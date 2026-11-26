(() => {
  "use strict";

  const revealDelay = window.matchMedia("(prefers-reduced-motion: reduce)").matches ? 0 : 80;
  const screenTransition = window.matchMedia("(prefers-reduced-motion: reduce)").matches ? 0 : 150;
  let revealTimeouts = [];

  const revealWords = (screen) => {
    revealTimeouts.forEach((timeout) => window.clearTimeout(timeout));
    revealTimeouts = [];

    const wordsContainer = screen.querySelector("[data-screen-words]");
    if (!wordsContainer) {
      console.error("Welcome screen text is unavailable.");
      return;
    }

    const words = wordsContainer.textContent.trim().split(/\s+/);
    const fragment = document.createDocumentFragment();
    words.forEach((word, index) => {
      if (index > 0) fragment.append(" ");
      const span = document.createElement("span");
      span.className = "welcome-word";
      const emphasizedWord = wordsContainer.dataset.emphasizeWord;
      if (emphasizedWord && word === `${emphasizedWord}.`) {
        const emphasizedText = document.createElement("span");
        emphasizedText.className = "lumas-font";
        emphasizedText.textContent = emphasizedWord;
        const period = document.createElement("span");
        period.className = "welcome-period";
        period.textContent = ".";
        span.append(emphasizedText, period);
      } else {
        span.textContent = word;
      }
      fragment.append(span);
    });
    wordsContainer.replaceChildren(fragment);

    wordsContainer.querySelectorAll(".welcome-word").forEach((word, index) => {
      revealTimeouts.push(
        window.setTimeout(() => word.classList.add("is-visible"), index * revealDelay),
      );
    });
  };

  const initializeWelcomeFlow = () => {
    const flow = document.querySelector("[data-welcome-flow]");
    if (!flow || flow.dataset.initialized === "true") return;
    flow.dataset.initialized = "true";

    const screens = [...flow.querySelectorAll("[data-welcome-screen]")];
    if (screens.length === 0) {
      console.error("Welcome screens are unavailable.");
      return;
    }

    screens.forEach((screen, index) => {
      screen.hidden = index !== 0;
    });
    revealWords(screens[0]);

    let activeIndex = 0;
    let transitioning = false;
    flow.addEventListener("click", (event) => {
      const screen = event.target instanceof Element
        ? event.target.closest("[data-welcome-screen]")
        : null;
      if (!screen || transitioning || screens[activeIndex] !== screen) return;

      const nextScreen = screens[activeIndex + 1];
      if (!nextScreen) {
        const finalLink = screen.querySelector("a.welcome-continue");
        if (finalLink && event.target !== finalLink) {
          event.preventDefault();
          finalLink.click();
        }
        return;
      }

      event.preventDefault();
      transitioning = true;
      const currentScreen = screens[activeIndex];
      currentScreen.classList.add("is-leaving");
      window.setTimeout(() => {
        currentScreen.hidden = true;
        currentScreen.classList.remove("is-leaving");
        activeIndex += 1;
        nextScreen.hidden = false;
        nextScreen.classList.add("is-entering");
        requestAnimationFrame(() => {
          requestAnimationFrame(() => nextScreen.classList.remove("is-entering"));
        });
        revealWords(nextScreen);
        window.setTimeout(() => {
          transitioning = false;
          nextScreen.querySelector(".welcome-continue")?.focus();
        }, screenTransition);
      }, screenTransition);
    });
  };

  initializeWelcomeFlow();
  document.addEventListener("site:pagecontent-updated", initializeWelcomeFlow);
})();
