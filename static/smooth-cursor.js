/*
 * Adapted from Magic UI Smooth Cursor (https://magicui.design/docs/components/smooth-cursor), original concept by @Code_Parth.
 * Copyright (c) Magic UI
 *
 * MIT License
 *
 * Permission is hereby granted, free of charge, to any person obtaining a copy
 * of this software and associated documentation files (the "Software"), to deal
 * in the Software without restriction, including without limitation the rights
 * to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
 * copies of the Software, and to permit persons to whom the Software is
 * furnished to do so, subject to the following conditions:
 *
 * The above copyright notice and this permission notice shall be included in all
 * copies or substantial portions of the Software.
 *
 * THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
 * IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
 * FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
 * AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
 * LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
 * OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
 * SOFTWARE.
 */

(() => {
  "use strict";

  const POINTER_QUERY = "(any-hover: hover) and (any-pointer: fine)";
  const REDUCED_MOTION_QUERY = "(prefers-reduced-motion: reduce)";
  const EXCLUDED_SELECTOR = [
    "textarea",
    "select",
    'input:not([type])',
    'input[type=""]',
    'input[type="text"]',
    'input[type="email"]',
    'input[type="password"]',
    'input[type="search"]',
    'input[type="url"]',
    'input[type="tel"]',
    'input[type="number"]',
    'input[type="checkbox"]',
    'input[type="radio"]',
    'input[type="range"]',
    'input[type="file"]',
    '[contenteditable]:not([contenteditable="false"])',
    "iframe",
    "embed",
    "object",
    "video",
    "[data-no-custom-cursor]",
  ].join(",");
  const INTERACTIVE_SELECTOR = 'a, button, [role="button"], summary, label, input[type="submit"]';

  let pointerQuery;
  let reducedMotionQuery;
  let cursor = null;
  let frameId = 0;
  let scaleTimeout = 0;
  let lastFrameTime = 0;
  let lastPointer = null;
  let lastPointerTime = 0;
  let directionOrigin = null;
  let directionOriginTime = 0;
  let pendingPointer = null;
  let initialized = false;
  let stopped = false;
  let needsSnap = true;
  let visible = false;
  let moving = false;
  let hovering = false;
  let previousAngle = null;

  const springs = {
    x: { position: 0, velocity: 0, target: 0, stiffness: 600, damping: 60, mass: 1},
    y: { position: 0, velocity: 0, target: 0, stiffness: 600, damping: 60, mass: 1 },
    rotation: { position: 0, velocity: 0, target: 0, stiffness: 300, damping: 60, mass: 1 },
    scale: { position: 1, velocity: 0, target: 1, stiffness: 500, damping: 35, mass: 1 },
  };

  const guard = (callback) => (...args) => {
    try {
      callback(...args);
    } catch (error) {
      fail(error);
    }
  };

  const safeTarget = (target) => target instanceof Element ? target : null;
  const closest = (target, selector) => safeTarget(target)?.closest(selector) ?? null;

  const updateScaleTarget = () => {
    springs.scale.target = moving ? 0.95 : hovering ? 1.08 : 1;
    requestFrame();
  };

  const setCursorVisible = (nextVisible) => {
    visible = nextVisible;
    if (!cursor) return;
    cursor.style.opacity = nextVisible ? "1" : "0";
    if (nextVisible) document.documentElement.classList.add("sc-active");
    else document.documentElement.classList.remove("sc-active");
  };

  const hideCursor = () => {
    setCursorVisible(false);
    needsSnap = true;
    pendingPointer = null;
    lastPointer = null;
    directionOrigin = null;
    previousAngle = null;
    hovering = false;
    moving = false;
    if (scaleTimeout) {
      clearTimeout(scaleTimeout);
      scaleTimeout = 0;
    }
    springs.scale.target = 1;
    if (frameId) {
      cancelAnimationFrame(frameId);
      frameId = 0;
    }
  };

  const isResting = (spring) =>
    Math.abs(spring.position - spring.target) <= 0.001 && Math.abs(spring.velocity) <= 0.01;

  const integrate = (spring, dt) => {
    const acceleration =
      (-spring.stiffness * (spring.position - spring.target) - spring.damping * spring.velocity) /
      spring.mass;
    spring.velocity += acceleration * dt;
    spring.position += spring.velocity * dt;
    if (isResting(spring)) {
      spring.position = spring.target;
      spring.velocity = 0;
    }
  };

  const wrapAngleDifference = (difference) => {
    while (difference > 180) difference -= 360;
    while (difference < -180) difference += 360;
    return difference;
  };

  const processPointer = (pointer) => {
    if (pointer.pointerType === "touch") {
      hideCursor();
      return;
    }
    if (pointer.excluded) {
      hideCursor();
      return;
    }

    const now = Number.isFinite(pointer.time) ? pointer.time : performance.now();
    const point = { x: pointer.x, y: pointer.y };
    if (needsSnap) {
      springs.x.position = point.x;
      springs.x.velocity = 0;
      springs.y.position = point.y;
      springs.y.velocity = 0;
      springs.x.target = point.x;
      springs.y.target = point.y;
      needsSnap = false;
    }

    let speed = 0;
    if (lastPointer) {
      const elapsed = Math.max(1, now - lastPointerTime);
      const vx = (point.x - lastPointer.x) / elapsed;
      const vy = (point.y - lastPointer.y) / elapsed;
      speed = Math.hypot(vx, vy);
    }

    if (directionOrigin) {
      const dx = point.x - directionOrigin.x;
      const dy = point.y - directionOrigin.y;
      const distance = Math.hypot(dx, dy);
      const elapsed = Math.max(1, now - directionOriginTime);
      const directionSpeed = distance / elapsed;
      // Accumulate sub-pixel events, but do not let tiny device jitter steer the cursor.
      if (distance >= 0.75) {
        if (directionSpeed > 0.1) {
          const angle = Math.atan2(dy, dx) * (180 / Math.PI) + 90;
          if (previousAngle === null) {
            springs.rotation.target += wrapAngleDifference(angle - springs.rotation.target);
          } else {
            springs.rotation.target += wrapAngleDifference(angle - previousAngle);
          }
          previousAngle = angle;
        }
        directionOrigin = point;
        directionOriginTime = now;
      }
    } else {
      directionOrigin = point;
      directionOriginTime = now;
    }

    if (speed > 0.1) {
      moving = true;
      updateScaleTarget();
      if (scaleTimeout) clearTimeout(scaleTimeout);
      scaleTimeout = window.setTimeout(guard(() => {
          moving = false;
          scaleTimeout = 0;
          updateScaleTarget();
      }), 150);
    }

    lastPointer = point;
    lastPointerTime = now;
    springs.x.target = point.x;
    springs.y.target = point.y;
    setCursorVisible(true);
  };

  const renderFrame = (time) => {
    frameId = 0;
    if (stopped || !cursor) return;

    try {
      if (pendingPointer) {
        const pointer = pendingPointer;
        pendingPointer = null;
        processPointer(pointer);
      }

      if (!visible) return;
      const dt = lastFrameTime ? Math.min((time - lastFrameTime) / 1000, 1 / 30) : 1 / 60;
      lastFrameTime = time;
      Object.values(springs).forEach((spring) => integrate(spring, dt));
      cursor.style.transform =
        `translate3d(${springs.x.position}px, ${springs.y.position}px, 0) ` +
        `translate(-50%, -50%) rotate(${springs.rotation.position}deg) scale(${springs.scale.position})`;

      if (!Object.values(springs).every(isResting) || pendingPointer) requestFrame();
    } catch (error) {
      fail(error);
    }
  };

  function requestFrame() {
    if (!frameId && !stopped) frameId = requestAnimationFrame(renderFrame);
  }

  const onPointerMove = (event) => {
    if (event.pointerType !== "mouse" && event.pointerType !== "pen" && event.pointerType !== "touch") return;
    pendingPointer = {
      x: event.clientX,
      y: event.clientY,
      time: event.timeStamp,
      pointerType: event.pointerType,
      excluded: Boolean(closest(event.target, EXCLUDED_SELECTOR)),
    };
    requestFrame();
  };

  const onPointerDown = (event) => {
    if (event.pointerType === "touch") hideCursor();
  };

  const onPointerOver = (event) => {
    if (event.pointerType === "touch") {
      hideCursor();
      return;
    }
    if (closest(event.target, EXCLUDED_SELECTOR)) {
      hideCursor();
      return;
    }
    hovering = Boolean(closest(event.target, INTERACTIVE_SELECTOR));
    updateScaleTarget();
  };

  const onPointerOut = (event) => {
    if (event.pointerType === "touch") return;
    const nextTarget = safeTarget(event.relatedTarget);
    if (nextTarget && closest(nextTarget, EXCLUDED_SELECTOR)) {
      hideCursor();
      return;
    }
    hovering = Boolean(nextTarget && closest(nextTarget, INTERACTIVE_SELECTOR));
    updateScaleTarget();
  };

  const onPointerLeave = () => hideCursor();
  const onVisibilityChange = () => {
    if (document.hidden) hideCursor();
  };

  const removePointerListeners = () => {
    window.removeEventListener("pointermove", guardedOnPointerMove);
    window.removeEventListener("pointerdown", guardedOnPointerDown);
    window.removeEventListener("pointerleave", guardedOnPointerLeave);
    document.removeEventListener("pointerover", guardedOnPointerOver);
    document.removeEventListener("pointerout", guardedOnPointerOut);
    document.removeEventListener("visibilitychange", guardedOnVisibilityChange);
  };

  const createCursor = () => {
    if (cursor) return;
    cursor = document.createElement("div");
    cursor.id = "smooth-cursor";
    cursor.className = "sc-cursor";
    cursor.setAttribute("aria-hidden", "true");
    cursor.innerHTML = '<svg xmlns="http://www.w3.org/2000/svg" width="50" height="54" viewBox="0 0 50 54" fill="none"><path d="M42.6817 41.1495L27.5103 6.79925C26.7269 5.02557 24.2082 5.02558 23.3927 6.79925L7.59814 41.1495C6.75833 42.9759 8.52712 44.8902 10.4125 44.1954L24.3757 39.0496C24.8829 38.8627 25.4385 38.8627 25.9422 39.0496L39.8121 44.1954C41.6849 44.8902 43.4884 42.9759 42.6817 41.1495Z"/><path d="M43.7146 40.6933L28.5431 6.34306C27.3556 3.65428 23.5772 3.69516 22.3668 6.32755L6.57226 40.6778C5.3134 43.4156 7.97238 46.298 10.803 45.2549L24.7662 40.109C25.0221 40.0147 25.2999 40.0156 25.5494 40.1082L39.4193 45.254C42.2261 46.2953 44.9254 43.4347 43.7146 40.6933Z"/></svg>';
    document.body.appendChild(cursor);
  };

  const disable = () => {
    removePointerListeners();
    hideCursor();
    cursor?.remove();
    cursor = null;
    initialized = false;
  };

  const enable = () => {
    if (initialized || stopped || !pointerQuery.matches || reducedMotionQuery.matches) return;
    createCursor();
    initialized = true;
    window.addEventListener("pointermove", guardedOnPointerMove, { passive: true });
    window.addEventListener("pointerdown", guardedOnPointerDown, { passive: true });
    window.addEventListener("pointerleave", guardedOnPointerLeave);
    document.addEventListener("pointerover", guardedOnPointerOver, { passive: true });
    document.addEventListener("pointerout", guardedOnPointerOut, { passive: true });
    document.addEventListener("visibilitychange", guardedOnVisibilityChange);
  };

  const syncAvailability = () => {
    if (pointerQuery.matches && !reducedMotionQuery.matches) enable();
    else disable();
  };

  const cleanup = () => {
    stopped = true;
    disable();
    if (pointerQuery) pointerQuery.removeEventListener("change", onPreferenceChange);
    if (reducedMotionQuery) reducedMotionQuery.removeEventListener("change", onPreferenceChange);
    document.documentElement.classList.remove("sc-active");
  };

  function fail(error) {
    if (stopped) return;
    console.warn("Smooth cursor disabled after an error; the native cursor has been restored.", error);
    cleanup();
  }

  const onPreferenceChange = guard(syncAvailability);
  const guardedOnPointerMove = guard(onPointerMove);
  const guardedOnPointerDown = guard(onPointerDown);
  const guardedOnPointerLeave = guard(onPointerLeave);
  const guardedOnPointerOver = guard(onPointerOver);
  const guardedOnPointerOut = guard(onPointerOut);
  const guardedOnVisibilityChange = guard(onVisibilityChange);

  try {
    pointerQuery = window.matchMedia(POINTER_QUERY);
    reducedMotionQuery = window.matchMedia(REDUCED_MOTION_QUERY);
    pointerQuery.addEventListener("change", onPreferenceChange);
    reducedMotionQuery.addEventListener("change", onPreferenceChange);
    syncAvailability();
  } catch (error) {
    fail(error);
  }
})();