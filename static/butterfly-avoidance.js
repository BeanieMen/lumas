(() => {
  const reducedMotion = window.matchMedia("(prefers-reduced-motion: reduce)");
  if (reducedMotion.matches) return;

  const states = new Map();
  const radius = 180;
  const maxDisplacement = 72;
  let pointer = null;
  let frameId = 0;
  let lastFrameTime = 0;

  const animate = (time) => {
    frameId = 0;
    const elapsed = lastFrameTime ? Math.min(time - lastFrameTime, 50) : 16;
    const smoothing = 1 - Math.exp(-elapsed / 140);
    const butterflies = document.querySelectorAll(".hero .butterfly");
    let hasDisplacedButterflies = false;

    for (const [index, butterfly] of butterflies.entries()) {
      let state = states.get(butterfly);
      if (!state) {
        state = { x: 0, y: 0 };
        states.set(butterfly, state);
      }

      const bounds = butterfly.getBoundingClientRect();
      const centerX = bounds.left + bounds.width / 2 - state.x;
      const centerY = bounds.top + bounds.height / 2 - state.y;
      let targetX = 0;
      let targetY = 0;

      if (pointer) {
        const awayX = centerX - pointer.x;
        const awayY = centerY - pointer.y;
        const distance = Math.hypot(awayX, awayY);

        if (distance < radius) {
          const angle = distance > 0 ? Math.atan2(awayY, awayX) : index * 2.1;
          const strength = (1 - distance / radius) ** 2;
          targetX = Math.cos(angle) * maxDisplacement * strength;
          targetY = Math.sin(angle) * maxDisplacement * strength;
        }
      }

      state.x += (targetX - state.x) * smoothing;
      state.y += (targetY - state.y) * smoothing;
      butterfly.style.translate = `${state.x}px ${state.y}px`;

      if (Math.abs(state.x) > 0.1 || Math.abs(state.y) > 0.1) {
        hasDisplacedButterflies = true;
      } else {
        state.x = 0;
        state.y = 0;
      }
    }

    lastFrameTime = time;
    if (pointer || hasDisplacedButterflies) {
      frameId = requestAnimationFrame(animate);
    }
  };

  const requestAnimation = () => {
    if (!frameId) frameId = requestAnimationFrame(animate);
  };

  document.addEventListener("pointermove", (event) => {
    if (event.pointerType === "touch") return;
    const hero = event.target instanceof Element && event.target.closest(".hero");
    pointer = hero ? { x: event.clientX, y: event.clientY } : null;
    requestAnimation();
  }, { passive: true });

  document.addEventListener("pointerleave", (event) => {
    if (!(event.target instanceof Element) || !event.target.matches(".hero")) return;
    pointer = null;
    requestAnimation();
  }, true);

  window.addEventListener("blur", () => {
    pointer = null;
    requestAnimation();
  });

  reducedMotion.addEventListener("change", (event) => {
    if (!event.matches) return;
    pointer = null;
    states.clear();
    cancelAnimationFrame(frameId);
    frameId = 0;
    document.querySelectorAll(".hero .butterfly").forEach((butterfly) => {
      butterfly.style.translate = "0px 0px";
    });
  });
})();