(() => {
  const sections = new Map();
  let observer;

  const clamp = (value, min = 0, max = 1) => Math.min(max, Math.max(min, value));
  const lerp = (from, to, progress) => from + (to - from) * progress;

  const mount = (section) => {
    if (sections.has(section)) return;

    const cards = [...section.querySelectorAll(".split-card")].slice(0, 4);
    const cardRow = section.querySelector(".split-cards");
    if (cards.length !== 4 || !cardRow) return;

    let frame = 0;
    const update = () => {
      frame = 0;
      const maxScroll = section.offsetHeight - window.innerHeight;
      const progress = maxScroll > 0
        ? clamp(-section.getBoundingClientRect().top / maxScroll)
        : 0;
      const separation = clamp(progress / 0.4);
      const flipping = clamp((progress - 0.4) / 0.4);
      const finish = clamp((progress - 0.8) / 0.2);
      const radius = `${lerp(0, 16, clamp(progress / 0.2))}px`;
      const shadowOpacity = lerp(0, 0.4, clamp(progress / 0.2));

      cards.forEach((card, index) => {
        const side = (index - 1.5) / 1.5;
        card.style.setProperty("--split-x", `${side * lerp(0, 48, separation)}px`);
        card.style.setProperty("--split-rotate-y", `${180 * flipping}deg`);
        card.style.setProperty("--split-rotate-z", `${side * 6 * flipping}deg`);
        card.style.setProperty("--split-radius", radius);
        card.style.setProperty(
          "--split-shadow",
          `inset 0 1px 1px rgba(255,255,255,${shadowOpacity * 0.5}), inset 0 -24px 48px rgba(0,0,0,${shadowOpacity}), 0 25px 50px -12px rgba(0,0,0,${shadowOpacity})`
        );
      });

      cardRow.style.setProperty("--split-cards-scale", String(lerp(1, 0.9, separation)));
      cardRow.style.setProperty("--split-cards-y", `${-200 * finish}px`);
      section.style.setProperty("--split-cue-opacity", String(1 - clamp(progress / 0.1)));
      section.style.setProperty("--split-cue-y", `${20 * clamp(progress / 0.1)}px`);
      section.style.setProperty("--split-ending-opacity", String(finish));
      section.style.setProperty("--split-ending-y", `${40 * (1 - finish)}px`);
    };

    const scheduleUpdate = () => {
      if (!frame) frame = window.requestAnimationFrame(update);
    };
    window.addEventListener("scroll", scheduleUpdate, { passive: true });
    window.addEventListener("resize", scheduleUpdate, { passive: true });
    update();

    sections.set(section, () => {
      window.removeEventListener("scroll", scheduleUpdate);
      window.removeEventListener("resize", scheduleUpdate);
      window.cancelAnimationFrame(frame);
      sections.delete(section);
    });
  };

  const syncSections = () => {
    for (const [section, cleanup] of sections) {
      if (!section.isConnected) cleanup();
    }
    document.querySelectorAll(".scroll-split-section").forEach(mount);
  };

  syncSections();
  observer = new MutationObserver(syncSections);
  observer.observe(document.documentElement, { childList: true, subtree: true });
})();
