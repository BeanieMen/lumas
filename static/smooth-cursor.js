/*
 * Adapted from Magic UI Smooth Cursor (https://magicui.design/docs/components/smooth-cursor), original concept by @Code_Parth.
 *
 * MIT License
 * Copyright (c) Magic UI
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
(function () {
  "use strict";

  try {
    const pointerQuery = window.matchMedia("(any-hover: hover) and (any-pointer: fine)");
    const motionQuery = window.matchMedia("(prefers-reduced-motion: reduce)");
    const root = document.documentElement;
    const controlSelector = [
      "textarea",
      "select",
      "[contenteditable]:not([contenteditable='false'])",
      "input[type='text']",
      "input[type='email']",
      "input[type='password']",
      "input[type='search']",
      "input[type='url']",
      "input[type='tel']",
      "input[type='number']",
      "input[type='checkbox']",
      "input[type='radio']",
      "input[type='range']",
      "input[type='file']",
    ].join(",");
    const hoverSelector = "a, button, [role='button'], summary, label, input[type='submit']";
    const optOutSelector = "[data-no-custom-cursor], iframe, embed, object";

    let cursor = null;
    let enabled = false;
    let visible = false;
    let hoverTarget = null;
    let lastPointer = null;
    let lastFrameTime = 0;
    let moveFrame = 0;
    let springFrame = 0;
    let scaleTimeout = 0;
    let previousAngle = 0;
    let accumulatedRotation = 0;
    let lastMoveTime = 0;
    let lastMoveX = 0;
    let lastMoveY = 0;

    const springs = {
      x: { position: 0, target: 0, velocity: 0, stiffness: 400, damping: 45, mass: 1 },
      y: { position: 0, target: 0, velocity: 0, stiffness: 400, damping: 45, mass: 1 },
      rotation: { position: 0, target: 0, velocity: 0, stiffness: 300, damping: 60, mass: 1 },
      scale: { position: 1, target: 1, velocity: 0, stiffness: 500, damping: 35, mass: 1 },
    };

    const svgMarkup =
      '<svg class="sc-arrow" xmlns="http://www.w3.org/2000/svg" width="50" height="54" viewBox="0 0 50 54" fill="none" aria-hidden="true">' +
      '<path d="M42.6817 41.1495L27.5103 6.79925C26.7269 5.02557 24.2082 5.02558 23.3927 6.79925L7.59814 41.1495C6.75833 42.9759 8.52712 44.8902 10.4125 44.1954L24.3757 39.0496C24.8829 38.8627 25.4385 38.8627 25.9422 39.0496L39.8121 44.1954C41.6849 44.8902 43.4884 42.9759 42.6817 41.1495Z" fill="var(--sc-fill, #fff)" />' +
      '<path d="M43.7146 40.6933L28.5431 6.34306C27.3556 3.65428 23.5772 3.69516 22.3668 6.32755L6.57226 40.6778C5.3134 43.4156 7.97238 46.298 10.803 45.2549L24.7662 40.109C25.0221 40.0147 25.2999 40.0156 25.5494 40.1082L39.4193 45.254C42.2261 46.2953 44.9254 43.4347 43.7146 40.6933Z" fill="none" stroke="var(--sc-stroke, #000)" stroke-width="2.25825" />' +
      "</svg>";

    function setVisible(nextVisible) {
      visible = nextVisible;
      if (cursor) cursor.classList.toggle("sc-visible", visible);
      root.classList.toggle("sc-active", visible);
    }

    function createCursor() {
      cursor = document.createElement("div");
      cursor.id = "smooth-cursor";
      cursor.className = "sc-cursor";
      cursor.setAttribute("aria-hidden", "true");
      cursor.innerHTML = svgMarkup;
      document.body.appendChild(cursor);
    }

    function isSuppressed(target) {
      return target instanceof Element && Boolean(target.closest(controlSelector + "," + optOutSelector));
    }

    function isHoverTarget(target) {
      return target instanceof Element ? target.closest(hoverSelector) : null;
    }

    function applyTransform() {
      if (!cursor) return;
      const { x, y, rotation, scale } = springs;
      cursor.style.transform =
        "translate3d(" + x.position + "px, " + y.position + "px, 0) translate(-50%, -50%) rotate(" +
        rotation.position + "deg) scale(" + scale.position + ")";
    }

    function settle(spring) {
      if (Math.abs(spring.target - spring.position) <= 0.001 && Math.abs(spring.velocity) <= 0.01) {
        spring.position = spring.target;
        spring.velocity = 0;
        return true;
      }
      return false;
    }

    function animateSprings(now) {
      try {
        const dt = Math.min(Math.max((now - lastFrameTime) / 1000, 0), 1 / 30);
        lastFrameTime = now;
        let allSettled = true;

        for (const spring of Object.values(springs)) {
          const acceleration =
            (-spring.stiffness * (spring.position - spring.target) - spring.damping * spring.velocity) /
            spring.mass;
          spring.velocity += acceleration * dt;
          spring.position += spring.velocity * dt;
          if (!settle(spring)) allSettled = false;
        }

        applyTransform();
        if (allSettled) {
          springFrame = 0;
          return;
        }
        springFrame = requestAnimationFrame(animateSprings);
      } catch (error) {
        failSafe(error);
      }
    }

    function startSpringLoop() {
      if (springFrame) return;
      lastFrameTime = performance.now();
      springFrame = requestAnimationFrame(animateSprings);
    }

    function processPointerMove() {
      moveFrame = 0;
      if (!enabled || !lastPointer) return;

      try {
        const { x, y, time, target, pointerType } = lastPointer;
        if (pointerType === "touch") {
          setVisible(false);
          hoverTarget = null;
          lastMoveTime = 0;
          if (cursor) cursor.classList.remove("sc-hover");
          return;
        }
        if (isSuppressed(target)) {
          setVisible(false);
          hoverTarget = null;
          lastMoveTime = 0;
          if (cursor) cursor.classList.remove("sc-hover");
          return;
        }

        const nextHoverTarget = isHoverTarget(target);
        if (nextHoverTarget !== hoverTarget) {
          hoverTarget = nextHoverTarget;
          cursor.classList.toggle("sc-hover", Boolean(hoverTarget));
        }

        if (!visible) {
          springs.x.position = springs.x.target = x;
          springs.y.position = springs.y.target = y;
          springs.x.velocity = springs.y.velocity = 0;
          applyTransform();
          setVisible(true);
        }

        springs.x.target = x;
        springs.y.target = y;
        if (lastMoveTime) {
          const elapsed = time - lastMoveTime;
          if (elapsed > 0) {
            const velocityX = (x - lastMoveX) / elapsed;
            const velocityY = (y - lastMoveY) / elapsed;
            const speed = Math.hypot(velocityX, velocityY);
            if (speed > 0.1) {
              const angle = (Math.atan2(velocityY, velocityX) * 180) / Math.PI + 90;
              let angleDifference = angle - previousAngle;
              if (angleDifference > 180) angleDifference -= 360;
              if (angleDifference < -180) angleDifference += 360;
              accumulatedRotation += angleDifference;
              springs.rotation.target = accumulatedRotation;
              previousAngle = angle;
              springs.scale.target = 0.95;
              window.clearTimeout(scaleTimeout);
              scaleTimeout = window.setTimeout(() => {
                springs.scale.target = 1;
                startSpringLoop();
              }, 150);
            }
          }
        }
        lastMoveTime = time;
        lastMoveX = x;
        lastMoveY = y;
        startSpringLoop();
      } catch (error) {
        failSafe(error);
      }
    }

    function onPointerMove(event) {
      try {
        if (event.pointerType === "touch") {
          lastPointer = { pointerType: "touch" };
        } else {
          lastPointer = {
            x: event.clientX,
            y: event.clientY,
            time: event.timeStamp || performance.now(),
            target: event.target,
            pointerType: event.pointerType,
          };
        }
        if (!moveFrame) moveFrame = requestAnimationFrame(processPointerMove);
      } catch (error) {
        failSafe(error);
      }
    }

    function onPointerOver(event) {
      try {
        if (event.pointerType === "touch" || isSuppressed(event.target)) {
          setVisible(false);
          hoverTarget = null;
          lastMoveTime = 0;
          if (cursor) cursor.classList.remove("sc-hover");
          return;
        }
        hoverTarget = isHoverTarget(event.target);
        if (cursor) cursor.classList.toggle("sc-hover", Boolean(hoverTarget));
      } catch (error) {
        failSafe(error);
      }
    }

    function onPointerOut(event) {
      try {
        if (event.relatedTarget === null) onPointerLeave();
      } catch (error) {
        failSafe(error);
      }
    }

    function onPointerLeave() {
      try {
        setVisible(false);
        hoverTarget = null;
        lastMoveTime = 0;
        if (cursor) cursor.classList.remove("sc-hover");
      } catch (error) {
        failSafe(error);
      }
    }

    function onVisibilityChange() {
      if (document.hidden) onPointerLeave();
    }

    function removeListeners() {
      window.removeEventListener("pointermove", onPointerMove);
      window.removeEventListener("pointerleave", onPointerLeave);
      window.removeEventListener("pointerout", onPointerOut);
      document.removeEventListener("pointerover", onPointerOver);
      document.removeEventListener("visibilitychange", onVisibilityChange);
    }

    function deactivate() {
      enabled = false;
      setVisible(false);
      removeListeners();
      if (moveFrame) cancelAnimationFrame(moveFrame);
      if (springFrame) cancelAnimationFrame(springFrame);
      if (scaleTimeout) clearTimeout(scaleTimeout);
      moveFrame = springFrame = scaleTimeout = 0;
      lastPointer = null;
      if (cursor) cursor.remove();
      cursor = null;
    }

    function activate() {
      if (enabled || !pointerQuery.matches || motionQuery.matches || !document.body) return;
      enabled = true;
      try {
        createCursor();
        window.addEventListener("pointermove", onPointerMove, { passive: true });
        window.addEventListener("pointerleave", onPointerLeave);
        window.addEventListener("pointerout", onPointerOut);
        document.addEventListener("pointerover", onPointerOver, { passive: true });
        document.addEventListener("visibilitychange", onVisibilityChange);
      } catch (error) {
        failSafe(error);
      }
    }

    function failSafe(error) {
      deactivate();
      pointerQuery.removeEventListener("change", onMediaChange);
      motionQuery.removeEventListener("change", onMediaChange);
      console.warn("Smooth cursor disabled; the native cursor remains available.", error);
    }

    function onMediaChange() {
      if (pointerQuery.matches && !motionQuery.matches) activate();
      else deactivate();
    }

    pointerQuery.addEventListener("change", onMediaChange);
    motionQuery.addEventListener("change", onMediaChange);
    onMediaChange();
  } catch (error) {
    document.documentElement.classList.remove("sc-active");
    const cursor = document.getElementById("smooth-cursor");
    if (cursor) cursor.remove();
    console.warn("Smooth cursor could not be initialized; the native cursor remains available.", error);
  }
})();