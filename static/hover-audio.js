const AUDIO_URL = "https://cdn.hackclub.com/01a17d1b-6d54-7d5a-ad71-3e16f791a479/WhatsApp%20Audio%202026-10-27%20at%202.33.01%20PM.mpeg"; // Set this to the CDN URL for the 16-clip MP3 sprite.
const STORAGE_KEY = "lumas-hover-audio-muted";
const SLOT_DURATION = 2.4;
const CLIP_SOUND_DURATION = 0.8;
const DEFAULT_VOLUME = 1;
const MAX_ACTIVE_SOUNDS = 3;
const GLOBAL_COOLDOWN_MS = 60;
const END_FADE_SECONDS = 0.03;
const EVICTION_FADE_SECONDS = 0.08;

let initialized = false;
let disabled = false;
let failureLogged = false;
let muted = false;
let audioContext = null;
let audioBuffer = null;
let loadingPromise = null;
let lastPlayTime = -Infinity;
let activeSounds = [];
let elementSounds = new WeakMap();
let devWarningShown = false;

const isDevelopment = () =>
  location.hostname === "localhost" ||
  location.hostname === "127.0.0.1" ||
  location.hostname.endsWith(".localhost");

const getClip = (element) => {
  const value = element.getAttribute("data-audio");
  if (value !== null && /^\d+$/.test(value)) {
    const clip = Number(value);
    if (clip >= 1 && clip <= 16) return clip;
  }

  if (value !== null && isDevelopment() && !devWarningShown) {
    console.warn("Hover audio: data-audio must be an integer from 1 to 16.");
    devWarningShown = true;
  }
  return null;
};

const getVolume = (element) => {
  const rawVolume = element.getAttribute("data-audio-volume");
  if (rawVolume === null) return DEFAULT_VOLUME;

  const volume = Number(rawVolume);
  return Number.isFinite(volume) ? Math.min(1, Math.max(0, volume)) : DEFAULT_VOLUME;
};

const isUnavailable = (element) =>
  document.hidden ||
  element.matches(":disabled") ||
  element.hasAttribute("disabled") ||
  element.getAttribute("aria-disabled") === "true";

const logFailureOnce = (error) => {
  disabled = true;
  if (!failureLogged) {
    console.warn("Hover audio is unavailable; the site will continue without sound.", error);
    failureLogged = true;
  }
};

const ensureLoaded = () => {
  if (disabled || audioBuffer || loadingPromise || !audioContext || !AUDIO_URL) return loadingPromise;

  const context = audioContext;
  loadingPromise = fetch(AUDIO_URL)
    .then((response) => {
      if (!response.ok) throw new Error(`Audio request failed (${response.status}).`);
      return response.arrayBuffer();
    })
    .then((data) => context.decodeAudioData(data))
    .then((buffer) => {
      if (audioContext === context && initialized) audioBuffer = buffer;
    })
    .catch((error) => {
      if (audioContext === context && initialized) logFailureOnce(error);
    })
    .finally(() => {
      loadingPromise = null;
    });

  return loadingPromise;
};

const initializeAudio = () => {
  if (!initialized || disabled) return;

  try {
    if (!audioContext) {
      if (navigator.audioSession && "type" in navigator.audioSession) {
        try {
          navigator.audioSession.type = "playback";
        } catch {
          // This is an optional browser feature.
        }
      }
      const AudioContextConstructor = window.AudioContext || window.webkitAudioContext;
      if (!AudioContextConstructor) return;
      audioContext = new AudioContextConstructor();
    }

    if (audioContext.state === "suspended") {
      audioContext.resume().catch(() => {});
    }
    void ensureLoaded();
  } catch (error) {
    logFailureOnce(error);
  }
};

const isSameElementSoundActive = (element, clip) => {
  const sound = elementSounds.get(element);
  return Boolean(
    sound &&
    sound.clip === clip &&
    sound.active &&
    performance.now() < sound.audibleUntil
  );
};

const fadeAndStop = (sound, fadeSeconds) => {
  if (!sound.active || sound.evicting) return;
  sound.evicting = true;
  const now = audioContext.currentTime;
  const stopAt = now + fadeSeconds;
  sound.gain.gain.cancelScheduledValues(now);
  sound.gain.gain.setValueAtTime(sound.gain.gain.value, now);
  sound.gain.gain.linearRampToValueAtTime(0, stopAt);
  sound.source.stop(stopAt);
};

const play = (element) => {
  if (!initialized || disabled || muted || isUnavailable(element)) return;

  const clip = getClip(element);
  if (!clip || isSameElementSoundActive(element, clip)) return;

  const nowMs = performance.now();
  if (nowMs - lastPlayTime < GLOBAL_COOLDOWN_MS) return;
  if (!audioContext || !audioBuffer || audioContext.state !== "running") return;

  const offset = (clip - 1) * SLOT_DURATION;
  if (offset >= audioBuffer.duration) return;
  const duration = Math.min(SLOT_DURATION, audioBuffer.duration - offset);
  if (duration <= 0) return;

  let scheduledStart = audioContext.currentTime;
  if (activeSounds.length >= MAX_ACTIVE_SOUNDS) {
    const oldest = activeSounds.find((sound) => sound.active && !sound.evicting);
    if (oldest) {
      scheduledStart += EVICTION_FADE_SECONDS;
      fadeAndStop(oldest, EVICTION_FADE_SECONDS);
    }
  }

  const source = audioContext.createBufferSource();
  const gain = audioContext.createGain();
  const volume = getVolume(element);
  const fadeStart = scheduledStart + Math.max(0, duration - END_FADE_SECONDS);
  const endTime = scheduledStart + duration;

  source.buffer = audioBuffer;
  gain.gain.setValueAtTime(volume, scheduledStart);
  gain.gain.setValueAtTime(volume, fadeStart);
  gain.gain.linearRampToValueAtTime(0, endTime);
  source.connect(gain);
  gain.connect(audioContext.destination);

  const sound = {
    source,
    gain,
    element,
    clip,
    active: true,
    evicting: false,
    audibleUntil: performance.now() +
      Math.max(0, scheduledStart - audioContext.currentTime) * 1000 +
      CLIP_SOUND_DURATION * 1000,
  };
  activeSounds.push(sound);
  elementSounds.set(element, sound);
  source.onended = () => {
    sound.active = false;
    activeSounds = activeSounds.filter((activeSound) => activeSound !== sound);
    if (elementSounds.get(element) === sound) elementSounds.delete(element);
    source.disconnect();
    gain.disconnect();
  };
  source.start(scheduledStart, offset, duration);
  lastPlayTime = nowMs;
};

const findAudioElement = (target) => {
  if (!(target instanceof Element)) return null;
  return target.closest("[data-audio]");
};

const onPointerOver = (event) => {
  if (event.pointerType === "touch") return;
  const element = findAudioElement(event.target);
  if (!element || element.getAttribute("data-audio-event") === "click") return;
  if (event.relatedTarget instanceof Node && element.contains(event.relatedTarget)) return;
  play(element);
};

const onPointerOut = (event) => {
  if (event.pointerType === "touch") return;
  const element = findAudioElement(event.target);
  if (!element || (event.relatedTarget instanceof Node && element.contains(event.relatedTarget))) return;
  // Sounds finish naturally when the pointer leaves.
};

const onClick = (event) => {
  const element = findAudioElement(event.target);
  if (element && element.getAttribute("data-audio-event") === "click") play(element);
};

const onFirstGesture = () => {
  document.removeEventListener("pointerdown", onFirstGesture);
  document.removeEventListener("keydown", onFirstGesture);
  document.removeEventListener("touchend", onFirstGesture);
  initializeAudio();
};

const readMutedPreference = () => {
  try {
    muted = localStorage.getItem(STORAGE_KEY) === "true";
  } catch {
    muted = false;
  }
};

const init = () => {
  if (initialized) return;
  initialized = true;
  disabled = false;
  readMutedPreference();
  document.addEventListener("pointerover", onPointerOver);
  document.addEventListener("pointerout", onPointerOut);
  document.addEventListener("click", onClick);
  document.addEventListener("pointerdown", onFirstGesture);
  document.addEventListener("keydown", onFirstGesture);
  document.addEventListener("touchend", onFirstGesture);
};

const destroy = () => {
  if (!initialized) return;
  initialized = false;
  document.removeEventListener("pointerover", onPointerOver);
  document.removeEventListener("pointerout", onPointerOut);
  document.removeEventListener("click", onClick);
  document.removeEventListener("pointerdown", onFirstGesture);
  document.removeEventListener("keydown", onFirstGesture);
  document.removeEventListener("touchend", onFirstGesture);
  for (const sound of activeSounds) {
    if (!sound.active) continue;
    sound.active = false;
    try {
      sound.source.stop();
      sound.source.disconnect();
      sound.gain.disconnect();
    } catch {
      // A source may already have stopped naturally.
    }
  }
  activeSounds = [];
  elementSounds = new WeakMap();
  audioBuffer = null;
  loadingPromise = null;
  const context = audioContext;
  audioContext = null;
  if (context && context.state !== "closed") void context.close().catch(() => {});
};

export const hoverAudio = {
  init,
  destroy,
  setMuted(value) {
    muted = Boolean(value);
    try {
      localStorage.setItem(STORAGE_KEY, String(muted));
    } catch {
      // Audio remains usable if storage is unavailable.
    }
  },
  isMuted() {
    return muted;
  },
};

if (typeof window !== "undefined") {
  window.hoverAudio = hoverAudio;
}
