use io_uring::{IoUring, cqueue, opcode, squeue, types};
use socket2::{Domain, Protocol, Socket, Type};
use std::{
    alloc::{Layout, alloc_zeroed},
    collections::VecDeque,
    net::SocketAddr,
    os::fd::AsRawFd,
    sync::atomic::{AtomicU16, Ordering},
};

use crate::model::{Asset, Blob, Site};

const RING_ENTRIES: u32 = 1024;
const BUF_SIZE: usize = 2048; // per-recv buffer; requests are tiny
const BUF_COUNT: usize = 1024; // power of two; shared by ALL connections on a thread
const BGID: u16 = 0;
const MAX_REQUEST: usize = 16 * 1024;

const R400: &[u8] = b"HTTP/1.1 400 Bad Request\r\nconnection: close\r\ncontent-length: 0\r\n\r\n";
const R404: &[u8] = b"HTTP/1.1 404 Not Found\r\ncontent-length: 0\r\n\r\n";
const R405: &[u8] = b"HTTP/1.1 405 Method Not Allowed\r\nallow: GET, HEAD\r\ncontent-length: 0\r\n\r\n";
const R431: &[u8] = b"HTTP/1.1 431 Request Header Fields Too Large\r\nconnection: close\r\ncontent-length: 0\r\n\r\n";

// ───────────────────────── HTTP ─────────────────────────

/// Does `Accept-Encoding` list `coding` with q > 0?
fn accepts(header: &str, coding: &str) -> bool {
    header.split(',').any(|part| {
        let mut it = part.split(';');
        if !it.next().unwrap_or("").trim().eq_ignore_ascii_case(coding) {
            return false;
        }
        it.all(|p| match p.trim().strip_prefix("q=") {
            Some(q) => q.trim().parse::<f32>().map_or(true, |q| q > 0.0),
            None => true,
        })
    })
}

/// Weak comparison against a possibly comma-separated `If-None-Match`.
fn etag_matches(inm: &[u8], etag: &[u8]) -> bool {
    let bare = |t: &[u8]| -> usize { if t.starts_with(b"W/") { 2 } else { 0 } };
    let e = &etag[bare(etag)..];
    inm.split(|&b| b == b',').any(|t| {
        let t = t.trim_ascii();
        t == b"*" || &t[bare(t)..] == e
    })
}

fn pick(a: &'static Asset, ae: &str, inm: &[u8], head_only: bool) -> &'static [u8] {
    if !inm.is_empty() && etag_matches(inm, a.etag) {
        return a.not_modified;
    }
    let b: &'static Blob = a
        .br
        .as_ref()
        .filter(|_| accepts(ae, "br"))
        .or_else(|| a.gzip.as_ref().filter(|_| accepts(ae, "gzip")))
        .unwrap_or(&a.plain);
    let bytes: &'static [u8] = b.bytes;
    if head_only { &bytes[..b.head] } else { bytes }
}

enum Parsed {
    Partial,
    Done { consumed: usize, resp: &'static [u8], close: bool },
}

fn contains_ci(hay: &[u8], needle: &[u8]) -> bool {
    hay.windows(needle.len()).any(|w| w.eq_ignore_ascii_case(needle))
}

fn parse(site: &Site, input: &[u8]) -> Parsed {
    let mut hdrs = [httparse::EMPTY_HEADER; 32];
    let mut req = httparse::Request::new(&mut hdrs);
    let n = match req.parse(input) {
        Ok(httparse::Status::Complete(n)) => n,
        Ok(httparse::Status::Partial) => return Parsed::Partial,
        Err(httparse::Error::TooManyHeaders) => {
            return Parsed::Done { consumed: input.len(), resp: R431, close: true };
        }
        Err(_) => return Parsed::Done { consumed: input.len(), resp: R400, close: true },
    };

    let mut close = req.version != Some(1); // HTTP/1.0: no keep-alive
    let mut ae = "";
    let mut inm: &[u8] = b"";
    for h in req.headers.iter() {
        if h.name.eq_ignore_ascii_case("accept-encoding") {
            ae = std::str::from_utf8(h.value).unwrap_or("");
        } else if h.name.eq_ignore_ascii_case("if-none-match") {
            inm = h.value;
        } else if h.name.eq_ignore_ascii_case("connection") {
            if contains_ci(h.value, b"close") {
                close = true;
            }
        } else if h.name.eq_ignore_ascii_case("transfer-encoding") {
            close = true; // bodies aren't parsed; don't desync on them
        } else if h.name.eq_ignore_ascii_case("content-length") && h.value.trim_ascii() != b"0" {
            close = true;
        }
    }

    let head_only = match req.method {
        Some("GET") => false,
        Some("HEAD") => true,
        _ => return Parsed::Done { consumed: n, resp: R405, close },
    };

    let target = req.path.unwrap_or("/");
    let path = target.split('?').next().unwrap_or("/");
    let resp = match site.lookup(path) {
        Some(a) => pick(a, ae, inm, head_only),
        None => R404,
    };
    Parsed::Done { consumed: n, resp, close }
}

// ───────────────────────── io_uring server ─────────────────────────

const OP_ACCEPT: u64 = 0;
const OP_RECV: u64 = 1;
const OP_SEND: u64 = 2;
const OP_SHUTDOWN: u64 = 3;
const OP_CLOSE: u64 = 4;

fn ud(op: u64, slot: usize) -> u64 {
    op | ((slot as u64) << 8)
}

#[derive(Default)]
struct Conn {
    fd: i32,
    live: bool,
    closing: bool,
    close_after: bool, // close once the output queue has drained
    sending: bool,
    inflight: u32,  // ops whose final CQE hasn't arrived yet
    off: usize,     // bytes of out.front() already sent
    inbuf: Vec<u8>, // only used when a request spans several recvs
    out: VecDeque<&'static [u8]>,
}

struct Server {
    ring: IoUring,
    site: &'static Site,
    lfd: i32,
    conns: Vec<Conn>,
    free: Vec<usize>,
    bufs: *mut u8,
    br_mem: *mut u8,
    br_tail: u16,
}

fn push(ring: &mut IoUring, e: &squeue::Entry) {
    loop {
        // SAFETY: all pointers in entries are 'static data or the provided-buffer pool.
        if unsafe { ring.submission().push(e) }.is_ok() {
            return;
        }
        ring.submit().ok(); // SQ full: flush and retry
    }
}

impl Server {
    fn new(site: &'static Site, lfd: i32) -> Self {
        let ring = IoUring::builder()
            .setup_single_issuer()
            .setup_defer_taskrun()
            .build(RING_ENTRIES)
            .or_else(|_| IoUring::new(RING_ENTRIES))
            .expect("io_uring unavailable (needs Linux >= 6.0 and not blocked by seccomp)");

        let bufs = unsafe { alloc_zeroed(Layout::from_size_align(BUF_SIZE * BUF_COUNT, 4096).unwrap()) };
        let br_mem = unsafe { alloc_zeroed(Layout::from_size_align(16 * BUF_COUNT, 4096).unwrap()) };
        unsafe {
            ring.submitter()
                .register_buf_ring_with_flags(br_mem as u64, BUF_COUNT as u16, BGID, 0)
                .expect("register_buf_ring failed");
        }

        let mut s = Server {
            ring,
            site,
            lfd,
            conns: Vec::new(),
            free: Vec::new(),
            bufs,
            br_mem,
            br_tail: 0,
        };
        for bid in 0..BUF_COUNT {
            s.give_back(bid as u16);
        }
        s.publish();
        s
    }

    // Entry layout (16 bytes): addr u64 @0, len u32 @8, bid u16 @12, resv u16 @14.
    fn give_back(&mut self, bid: u16) {
        let idx = (self.br_tail as usize) & (BUF_COUNT - 1);
        unsafe {
            let e = self.br_mem.add(idx * 16);
            (e as *mut u64).write(self.bufs as u64 + bid as u64 * BUF_SIZE as u64);
            (e.add(8) as *mut u32).write(BUF_SIZE as u32);
            (e.add(12) as *mut u16).write(bid);
        }
        self.br_tail = self.br_tail.wrapping_add(1);
    }

    // The ring tail lives in the `resv` field of entry 0 (byte offset 14).
    fn publish(&self) {
        unsafe { (*(self.br_mem.add(14) as *const AtomicU16)).store(self.br_tail, Ordering::Release) };
    }

    fn arm_accept(&mut self) {
        let e = opcode::AcceptMulti::new(types::Fd(self.lfd)).build().user_data(ud(OP_ACCEPT, 0));
        push(&mut self.ring, &e);
    }

    fn arm_recv(&mut self, slot: usize) {
        let fd = self.conns[slot].fd;
        self.conns[slot].inflight += 1;
        let e = opcode::RecvMulti::new(types::Fd(fd), BGID)
            .build()
            .flags(squeue::Flags::BUFFER_SELECT)
            .user_data(ud(OP_RECV, slot));
        push(&mut self.ring, &e);
    }

    fn run(&mut self) -> ! {
        self.arm_accept();
        let mut batch: Vec<(u64, i32, u32)> = Vec::with_capacity(1024);
        loop {
            if let Err(e) = self.ring.submit_and_wait(1) {
                if e.raw_os_error() == Some(libc::EINTR) || e.raw_os_error() == Some(libc::EBUSY) {
                    continue;
                }
                panic!("io_uring submit_and_wait: {e}");
            }
            batch.clear();
            batch.extend(self.ring.completion().map(|c| (c.user_data(), c.result(), c.flags())));
            for &(u, res, flags) in &batch {
                let slot = (u >> 8) as usize;
                match u & 0xff {
                    OP_ACCEPT => self.on_accept(res, flags),
                    OP_RECV => self.on_recv(slot, res, flags),
                    OP_SEND => self.on_send(slot, res),
                    _ => {
                        // shutdown / close
                        self.conns[slot].inflight -= 1;
                        self.maybe_free(slot);
                    }
                }
            }
            self.publish(); // hand returned buffers back to the kernel once per batch
        }
    }

    fn on_accept(&mut self, res: i32, flags: u32) {
        if res >= 0 {
            let slot = match self.free.pop() {
                Some(s) => s,
                None => {
                    self.conns.push(Conn::default());
                    self.conns.len() - 1
                }
            };
            let c = &mut self.conns[slot];
            c.fd = res;
            c.live = true;
            c.closing = false;
            c.close_after = false;
            c.sending = false;
            c.inflight = 0;
            c.off = 0;
            self.arm_recv(slot);
        }
        if !cqueue::more(flags) {
            self.arm_accept();
        }
    }

    fn on_recv(&mut self, slot: usize, res: i32, flags: u32) {
        let more = cqueue::more(flags);
        let bid = cqueue::buffer_select(flags);
        if !more {
            self.conns[slot].inflight -= 1;
        }

        if res > 0 {
            let bid = bid.expect("recv completed without a buffer id");
            // SAFETY: the buffer isn't returned to the kernel until after process().
            let data = unsafe {
                std::slice::from_raw_parts(self.bufs.add(bid as usize * BUF_SIZE), res as usize)
            };
            self.process(slot, data);
            self.give_back(bid);
        } else if let Some(b) = bid {
            self.give_back(b);
        }

        if res == 0 {
            self.conns[slot].close_after = true; // peer finished sending
            self.pump(slot);
        } else if res < 0 && res != -libc::ENOBUFS {
            self.begin_close(slot);
        }

        let c = &self.conns[slot];
        if !more && !c.closing && !c.close_after && (res > 0 || res == -libc::ENOBUFS) {
            self.arm_recv(slot); // multishot ended (e.g. buffers ran out): re-arm
        }
        self.maybe_free(slot);
    }

    fn process(&mut self, slot: usize, data: &[u8]) {
        let site = self.site;
        let c = &mut self.conns[slot];
        if c.closing || c.close_after {
            return;
        }

        let mut owned = std::mem::take(&mut c.inbuf);
        let pending = !owned.is_empty();
        if pending {
            owned.extend_from_slice(data);
        }
        let mut input: &[u8] = if pending { &owned } else { data };

        while !input.is_empty() {
            match parse(site, input) {
                Parsed::Partial => {
                    if input.len() > MAX_REQUEST {
                        c.out.push_back(R431);
                        c.close_after = true;
                        input = &[];
                    }
                    break;
                }
                Parsed::Done { consumed, resp, close } => {
                    c.out.push_back(resp);
                    input = &input[consumed..];
                    if close {
                        c.close_after = true;
                        input = &[];
                        break;
                    }
                }
            }
        }

        // Cold path: keep the unfinished tail for the next recv.
        if input.is_empty() {
            owned.clear();
            c.inbuf = owned;
        } else {
            c.inbuf = input.to_vec();
        }

        self.pump(slot);
    }

    // One in-flight send per connection; extra pipelined responses queue behind it.
    fn pump(&mut self, slot: usize) {
        let c = &mut self.conns[slot];
        if c.sending || c.closing {
            return;
        }
        if let Some(front) = c.out.front() {
            let rem = &front[c.off..];
            let e = opcode::Send::new(types::Fd(c.fd), rem.as_ptr(), rem.len() as u32)
                .flags(libc::MSG_NOSIGNAL)
                .build()
                .user_data(ud(OP_SEND, slot));
            c.sending = true;
            c.inflight += 1;
            push(&mut self.ring, &e);
        } else if c.close_after {
            self.begin_close(slot);
        }
    }

    fn on_send(&mut self, slot: usize, res: i32) {
        let c = &mut self.conns[slot];
        c.inflight -= 1;
        c.sending = false;
        if res <= 0 {
            self.begin_close(slot);
        } else {
            c.off += res as usize;
            if let Some(f) = c.out.front() {
                if c.off >= f.len() {
                    c.out.pop_front();
                    c.off = 0;
                }
            }
            self.pump(slot);
        }
        self.maybe_free(slot);
    }

    // shutdown wakes any pending recv; HARDLINK makes close run even if shutdown fails (ENOTCONN).
    fn begin_close(&mut self, slot: usize) {
        let c = &mut self.conns[slot];
        if c.closing {
            return;
        }
        c.closing = true;
        c.out.clear();
        c.inflight += 2;
        let sd = opcode::Shutdown::new(types::Fd(c.fd), libc::SHUT_RDWR)
            .build()
            .flags(squeue::Flags::IO_HARDLINK)
            .user_data(ud(OP_SHUTDOWN, slot));
        let cl = opcode::Close::new(types::Fd(c.fd)).build().user_data(ud(OP_CLOSE, slot));
        push(&mut self.ring, &sd);
        push(&mut self.ring, &cl);
    }

    // A slot is reused only after every CQE that references it has been seen.
    fn maybe_free(&mut self, slot: usize) {
        let c = &mut self.conns[slot];
        if c.live && c.closing && c.inflight == 0 {
            c.live = false;
            c.out.clear();
            c.inbuf.clear();
            self.free.push(slot);
        }
    }
}

// ───────────────────────── threads ─────────────────────────

fn listener(addr: SocketAddr) -> std::net::TcpListener {
    let s = Socket::new(Domain::for_address(addr), Type::STREAM, Some(Protocol::TCP)).unwrap();
    s.set_reuse_address(true).unwrap();
    s.set_reuse_port(true).unwrap();
    s.set_nodelay(true).unwrap(); // inherited by accepted sockets on Linux
    s.bind(&addr.into()).expect("failed to bind to address");
    s.listen(4096).unwrap();
    s.into()
}

fn pin(core: usize) {
    unsafe {
        let mut set: libc::cpu_set_t = std::mem::zeroed();
        libc::CPU_SET(core, &mut set);
        let _ = libc::sched_setaffinity(0, std::mem::size_of::<libc::cpu_set_t>(), &set);
    }
}

fn worker(site: &'static Site, addr: SocketAddr, core: usize) {
    pin(core);
    let l = listener(addr);
    Server::new(site, l.as_raw_fd()).run();
}

pub fn run() {
    let site: &'static Site = Box::leak(Box::new(Site::load()));
    let addr: SocketAddr = "127.0.0.1:3000".parse().unwrap();
    let n = std::thread::available_parallelism().map_or(1, |n| n.get());
    println!("[prod] http://{addr} ({n} io_uring threads)");
    let handles: Vec<_> = (0..n)
        .map(|i| std::thread::spawn(move || worker(site, addr, i)))
        .collect();
    for h in handles {
        let _ = h.join();
    }
}
