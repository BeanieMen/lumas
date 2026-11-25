(() => {
  const content = document.getElementById("page-content");
  if (!content) return;

  const updateStylesheet = (currentStyles, nextStyles, baseUrl) => {
    if (!currentStyles || !nextStyles) {
      return Promise.resolve(currentStyles === nextStyles);
    }

    const nextHref = new URL(nextStyles.getAttribute("href"), baseUrl).href;
    if (currentStyles.href === nextHref) return Promise.resolve(true);

    return new Promise((resolve) => {
      currentStyles.addEventListener("load", () => resolve(true), { once: true });
      currentStyles.addEventListener("error", () => resolve(false), { once: true });
      currentStyles.href = nextHref;
    });
  };

  const loadPage = async (url, addHistoryEntry) => {
    let response;
    try {
      response = await fetch(url, {
        headers: { Accept: "text/html" },
        credentials: "same-origin",
      });
    } catch (error) {
      console.error("Page navigation failed; loading the destination normally.", error);
      window.location.assign(url);
      return;
    }

    if (!response.ok) {
      console.error(`Page navigation failed with status ${response.status}; loading the destination normally.`);
      window.location.assign(url);
      return;
    }

    const page = new DOMParser().parseFromString(await response.text(), "text/html");
    const nextContent = page.getElementById("page-content");
    if (!nextContent) {
      console.error("Page navigation response did not contain #page-content; loading the destination normally.");
      window.location.assign(url);
      return;
    }

    const currentStyles = document.getElementById("page-styles");
    const nextStyles = page.getElementById("page-styles");
    if (Boolean(currentStyles) !== Boolean(nextStyles)) {
      console.error("Page navigation response has inconsistent page styles; loading the destination normally.");
      window.location.assign(url);
      return;
    }

    if (!await updateStylesheet(currentStyles, nextStyles, url)) {
      console.error("Page stylesheet failed to load; loading the destination normally.");
      window.location.assign(url);
      return;
    }

    content.replaceChildren(...nextContent.childNodes);
    document.title = page.title;
    if (addHistoryEntry) history.pushState({}, "", url);
    window.scrollTo(0, 0);
    if (window.AOS) {
      window.AOS.refreshHard();
      window.AOS.refresh();
      requestAnimationFrame(() => {
        window.dispatchEvent(new Event("scroll"));
      });
    }
  };

  document.addEventListener("click", (event) => {
    if (
      event.defaultPrevented ||
      event.button !== 0 ||
      event.metaKey ||
      event.ctrlKey ||
      event.shiftKey ||
      event.altKey
    ) return;

    const link = event.target instanceof Element ? event.target.closest("a[href]") : null;
    if (!link || !content.contains(link) || link.target || link.hasAttribute("download")) return;

    const url = new URL(link.href, window.location.href);
    if (
      url.origin !== window.location.origin ||
      url.pathname.startsWith("/auth/") ||
      url.pathname === window.location.pathname
    ) return;

    event.preventDefault();
    void loadPage(url.href, true);
  });

  window.addEventListener("popstate", () => {
    void loadPage(window.location.href, false);
  });
})();
