(() => {
  "use strict";

  const prefersReducedMotion = window.matchMedia("(prefers-reduced-motion: reduce)").matches;
  const transitionDuration = prefersReducedMotion ? 0 : 350;
  const stepFadeDuration = transitionDuration / 2;
  const welcomeBackDuration = prefersReducedMotion ? 0 : 900;
  const browserLoginKey = "lumas-browser-login-complete";
  const signup = {};
  let transitioning = false;

  const hasLoggedInInThisBrowser = () => {
    try {
      return localStorage.getItem(browserLoginKey) === "true";
    } catch (error) {
      console.warn("Browser login state could not be read.", error);
      return false;
    }
  };

  const markBrowserLoginComplete = () => {
    try {
      localStorage.setItem(browserLoginKey, "true");
    } catch (error) {
      console.warn("Browser login state could not be saved.", error);
    }
  };

  const currentUrl = new URL(window.location.href);
  if (currentUrl.searchParams.get("auth") === "success") {
    markBrowserLoginComplete();
    currentUrl.searchParams.delete("auth");
    history.replaceState({}, "", currentUrl);
  }

  const activateFlow = () => {
    document
      .querySelectorAll("#page-content > *:not(#signup-flow), body > footer")
      .forEach((element) => element.setAttribute("aria-hidden", "true"));
    document.body.classList.add("signup-flow-active");
  };

  const fillVerificationForm = () => {
    const form = document.querySelector('[data-signup-step="verify"]');
    if (!(form instanceof HTMLFormElement)) {
      console.error("Signup verification step is unavailable.");
      return false;
    }

    for (const fieldName of ["email", "name", "age"]) {
      const field = form.elements.namedItem(fieldName);
      if (!(field instanceof HTMLInputElement)) {
        console.error(`Signup verification field "${fieldName}" is unavailable.`);
        return false;
      }

      const value = signup[fieldName];
      field.value = value ?? "";
      field.disabled = value === undefined;
    }
    return true;
  };

  const startFirstBrowserLogin = () => {
    if (!document.getElementById("signup-flow")) {
      window.location.assign("/?collect-details=1");
      return;
    }
    activateFlow();
    showStep("name");
  };

  const welcomeBackAndRedirect = (loginUrl) => {
    activateFlow();
    showStep("welcome-back");
    window.setTimeout(() => window.location.assign(loginUrl), welcomeBackDuration);
  };

  const loginUrlWithEmail = () => {
    const loginUrl = new URL("/auth/hackclub/login", window.location.origin);
    if (signup.email) loginUrl.searchParams.set("email", signup.email);
    return loginUrl.href;
  };

  const showStep = (stepName) => {
    const flow = document.getElementById("signup-flow");
    const nextStep = flow?.querySelector(`[data-signup-step="${stepName}"]`);
    if (!flow || !nextStep) {
      console.error(`Signup flow step "${stepName}" is unavailable.`);
      return;
    }

    flow.setAttribute("aria-hidden", "false");
    const currentStep = flow.querySelector(".signup-step:not([hidden])");
    transitioning = true;

    const displayNextStep = () => {
      nextStep.hidden = false;
      nextStep.setAttribute("aria-hidden", "false");
      nextStep.classList.add("is-entering");
      requestAnimationFrame(() => {
        requestAnimationFrame(() => nextStep.classList.remove("is-entering"));
      });
      window.setTimeout(() => {
        transitioning = false;
        nextStep.querySelector("input, button, a")?.focus();
      }, stepFadeDuration);
    };

    if (currentStep) {
      currentStep.classList.add("is-leaving");
      window.setTimeout(() => {
        currentStep.hidden = true;
        currentStep.setAttribute("aria-hidden", "true");
        currentStep.classList.remove("is-leaving");
        displayNextStep();
      }, stepFadeDuration);
      return;
    }

    displayNextStep();
  };

  const collectDetailsParam = currentUrl.searchParams.get("collect-details") === "1";
  if (collectDetailsParam) {
    currentUrl.searchParams.delete("collect-details");
    history.replaceState({}, "", currentUrl);
    startFirstBrowserLogin();
  }

  document.addEventListener("submit", (event) => {
    if (!(event.target instanceof HTMLFormElement)) return;
    const form = event.target;
    if (transitioning) {
      event.preventDefault();
      return;
    }

    if (form.matches(".gh")) {
      event.preventDefault();
      const email = form.querySelector("#signup-email");
      if (!(email instanceof HTMLInputElement)) {
        console.error("Signup email field is unavailable.");
        return;
      }

      email.value = email.value.trim();
      if (!email.checkValidity()) {
        email.reportValidity();
        return;
      }

      signup.email = email.value;
      activateFlow();
      if (hasLoggedInInThisBrowser()) {
        welcomeBackAndRedirect(loginUrlWithEmail());
        return;
      }
      showStep("name");
      return;
    }

    if (form.matches('[data-signup-step="name"]')) {
      event.preventDefault();
      const name = form.querySelector("#signup-name");
      if (!(name instanceof HTMLInputElement)) {
        console.error("Signup name field is unavailable.");
        return;
      }

      name.value = name.value.trim();
      if (!form.checkValidity()) {
        form.reportValidity();
        return;
      }

      signup.name = name.value;
      showStep("age");
      return;
    }

    if (form.matches('[data-signup-step="age"]')) {
      event.preventDefault();
      const age = form.querySelector("#signup-age");
      if (!(age instanceof HTMLInputElement)) {
        console.error("Signup age field is unavailable.");
        return;
      }

      if (!form.checkValidity()) {
        form.reportValidity();
        return;
      }

      const ageValue = Number(age.value);
      if (ageValue >= 19) {
        showStep("ineligible");
        return;
      }

      signup.age = age.value;
      if (fillVerificationForm()) showStep("verify");
    }
  });

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
    if (!link) return;
    const targetUrl = new URL(link.href, window.location.href);
    if (
      targetUrl.origin !== window.location.origin ||
      targetUrl.pathname !== "/auth/hackclub/login"
    ) return;

    event.preventDefault();
    if (hasLoggedInInThisBrowser()) {
      welcomeBackAndRedirect(targetUrl.href);
      return;
    }
    startFirstBrowserLogin();
  });
})();
