(() => {
  "use strict";

  const vertexShaderSource = `
    attribute vec2 position;

    void main() {
      gl_Position = vec4(position, 0.0, 1.0);
    }
  `;

  const fragmentShaderSource = `
    precision mediump float;

    uniform vec2 resolution;
    uniform vec2 pointer;
    uniform float time;

    float randomValue(vec2 point) {
      point = fract(point * vec2(123.34, 456.21));
      point += dot(point, point + 45.32);
      return fract(point.x * point.y);
    }

    float smoothNoise(vec2 point) {
      vec2 cell = floor(point);
      vec2 local = fract(point);
      vec2 curve = local * local * local * (local * (local * 6.0 - 15.0) + 10.0);
      float lower = mix(randomValue(cell), randomValue(cell + vec2(1.0, 0.0)), curve.x);
      float upper = mix(randomValue(cell + vec2(0.0, 1.0)), randomValue(cell + vec2(1.0, 1.0)), curve.x);
      return mix(lower, upper, curve.y);
    }

    float layeredNoise(vec2 point) {
      float value = 0.0;
      float weight = 0.55;

      for (int octave = 0; octave < 5; octave++) {
        value += smoothNoise(point) * weight;
        point = mat2(0.78, -0.62, 0.62, 0.78) * point * 2.04 + 13.7;
        weight *= 0.48;
      }

      return value;
    }

    void main() {
      vec2 uv = gl_FragCoord.xy / resolution;
      float aspect = resolution.x / max(resolution.y, 1.0);
      vec2 point = (uv - 0.5) * vec2(aspect, 1.0);
      float drift = time * 0.1;

      vec2 pointerPoint = (pointer - 0.5) * vec2(aspect, 1.0);
      vec2 pointerOffset = pointerPoint - point;
      float pointerInfluence = exp(-length(pointerOffset) * 2.2);
      point += vec2(pointerOffset.y, -pointerOffset.x) * pointerInfluence * 0.055;

      vec2 movingPoint = point * 1.3 + vec2(drift * 0.42, -drift * 0.23);
      float warpHorizontal = layeredNoise(movingPoint + vec2(3.1, -1.7));
      float warpVertical = layeredNoise(movingPoint + vec2(-2.2, 4.3));
      vec2 warpedPoint = movingPoint + (vec2(warpHorizontal, warpVertical) - 0.5) * 0.95;

      float cloud = layeredNoise(warpedPoint * 0.88 + vec2(-drift * 0.18, drift * 0.31));
      float detail = layeredNoise(warpedPoint * 1.75 + vec2(7.4, -3.8));
      float ribbonPhase = warpedPoint.y * 3.2 + warpedPoint.x * 1.15
        + (warpHorizontal - warpVertical) * 4.0 + drift * 0.8;
      float ribbon = 0.5 + 0.5 * sin(ribbonPhase);
      float silk = smoothstep(0.46, 0.88, ribbon) * smoothstep(0.3, 0.7, cloud);
      float haze = smoothstep(0.22, 0.72, cloud * 0.72 + detail * 0.28);
      float broadFlow = smoothstep(0.24, 0.68, cloud);
      float brightFold = smoothstep(0.46, 0.84, detail) * broadFlow;

      float light = 0.018 + haze * 0.1 + broadFlow * 0.12;
      light += silk * 0.25 + brightFold * 0.14;

      vec2 edgePoint = (uv - 0.5) * vec2(1.0, 0.86);
      float edgeShade = 1.0 - smoothstep(0.22, 0.82, length(edgePoint)) * 0.34;
      float grain = (randomValue(gl_FragCoord.xy + floor(time * 9.0)) - 0.5) * 0.012;
      light = clamp(light * edgeShade + grain, 0.008, 0.5);

      gl_FragColor = vec4(vec3(light), 1.0);
    }
  `;

  const prefersReducedMotion = window.matchMedia("(prefers-reduced-motion: reduce)");
  const activeCanvases = new Set();

  const createShader = (context, type, source) => {
    const shader = context.createShader(type);
    if (!shader) {
      console.error("Aurora background shader could not be created.");
      return null;
    }

    context.shaderSource(shader, source);
    context.compileShader(shader);
    if (!context.getShaderParameter(shader, context.COMPILE_STATUS)) {
      console.error("Aurora background shader failed to compile:", context.getShaderInfoLog(shader));
      context.deleteShader(shader);
      return null;
    }
    return shader;
  };

  const mountAurora = (canvas) => {
    const host = canvas.parentElement;
    if (!host) {
      console.error("Aurora background has no containing element.");
      return () => {};
    }

    const context = canvas.getContext("webgl", {
      alpha: false,
      antialias: false,
      depth: false,
      stencil: false,
      powerPreference: "low-power",
    });
    if (!context) {
      console.warn("WebGL is unavailable; using the solid black background.");
      return () => {};
    }

    const vertexShader = createShader(context, context.VERTEX_SHADER, vertexShaderSource);
    const fragmentShader = createShader(context, context.FRAGMENT_SHADER, fragmentShaderSource);
    if (!vertexShader || !fragmentShader) {
      if (vertexShader) context.deleteShader(vertexShader);
      if (fragmentShader) context.deleteShader(fragmentShader);
      return () => {};
    }

    const program = context.createProgram();
    if (!program) {
      console.error("Aurora background WebGL program could not be created.");
      return () => {};
    }

    context.attachShader(program, vertexShader);
    context.attachShader(program, fragmentShader);
    context.linkProgram(program);
    context.deleteShader(vertexShader);
    context.deleteShader(fragmentShader);
    if (!context.getProgramParameter(program, context.LINK_STATUS)) {
      console.error("Aurora background WebGL program failed to link:", context.getProgramInfoLog(program));
      context.deleteProgram(program);
      return () => {};
    }

    const positionLocation = context.getAttribLocation(program, "position");
    const resolutionLocation = context.getUniformLocation(program, "resolution");
    const pointerLocation = context.getUniformLocation(program, "pointer");
    const timeLocation = context.getUniformLocation(program, "time");
    const vertices = context.createBuffer();
    if (positionLocation < 0 || !resolutionLocation || !pointerLocation || !timeLocation || !vertices) {
      console.error("Aurora background shader is missing a required WebGL binding.");
      context.deleteProgram(program);
      if (vertices) context.deleteBuffer(vertices);
      return () => {};
    }

    context.useProgram(program);
    context.bindBuffer(context.ARRAY_BUFFER, vertices);
    context.bufferData(
      context.ARRAY_BUFFER,
      new Float32Array([-1, -1, 1, -1, -1, 1, 1, 1]),
      context.STATIC_DRAW,
    );
    context.enableVertexAttribArray(positionLocation);
    context.vertexAttribPointer(positionLocation, 2, context.FLOAT, false, 0, 0);

    let pointerX = 0.5;
    let pointerY = 0.5;
    let width = 0;
    let height = 0;
    let animationFrame = 0;
    let visible = true;
    let running = true;
    const signupFlow = canvas.closest("#signup-flow");
    const startedAt = performance.now();

    const canDraw = () => (
      running &&
      visible &&
      (!signupFlow || document.body.classList.contains("signup-flow-active"))
    );

    const updateRendering = () => {
      if (!canDraw()) {
        if (animationFrame) {
          window.cancelAnimationFrame(animationFrame);
          animationFrame = 0;
        }
        return;
      }

      if (!animationFrame) draw(performance.now());
    };

    const resize = () => {
      const pixelRatio = Math.min(window.devicePixelRatio || 1, 1.5);
      const nextWidth = Math.max(1, Math.round(host.clientWidth * pixelRatio));
      const nextHeight = Math.max(1, Math.round(host.clientHeight * pixelRatio));
      if (nextWidth === width && nextHeight === height) return;

      width = nextWidth;
      height = nextHeight;
      canvas.width = width;
      canvas.height = height;
      context.viewport(0, 0, width, height);
      updateRendering();
    };

    const draw = (now) => {
      if (!canDraw()) return;
      context.useProgram(program);
      context.uniform2f(resolutionLocation, width, height);
      context.uniform2f(pointerLocation, pointerX, pointerY);
      context.uniform1f(timeLocation, prefersReducedMotion.matches ? 0 : (now - startedAt) / 1000);
      context.drawArrays(context.TRIANGLE_STRIP, 0, 4);
      if (!prefersReducedMotion.matches) animationFrame = window.requestAnimationFrame(draw);
    };

    const onPointerMove = (event) => {
      const bounds = host.getBoundingClientRect();
      pointerX = (event.clientX - bounds.left) / Math.max(bounds.width, 1);
      pointerY = 1 - (event.clientY - bounds.top) / Math.max(bounds.height, 1);
    };

    const resizeObserver = new ResizeObserver(resize);
    resizeObserver.observe(host);
    host.addEventListener("pointermove", onPointerMove, { passive: true });

    const visibilityObserver = new IntersectionObserver(([entry]) => {
      visible = entry.isIntersecting;
      updateRendering();
    });
    visibilityObserver.observe(canvas);

    const signupFlowObserver = signupFlow
      ? new MutationObserver(updateRendering)
      : null;
    signupFlowObserver?.observe(document.body, { attributes: true, attributeFilter: ["class"] });

    resize();
    updateRendering();

    return () => {
      running = false;
      if (animationFrame) window.cancelAnimationFrame(animationFrame);
      resizeObserver.disconnect();
      visibilityObserver.disconnect();
      signupFlowObserver?.disconnect();
      host.removeEventListener("pointermove", onPointerMove);
      context.deleteBuffer(vertices);
      context.deleteProgram(program);
    };
  };

  const initializeAurora = () => {
    for (const mounted of activeCanvases) {
      if (!mounted.canvas.isConnected) {
        mounted.unmount();
        activeCanvases.delete(mounted);
      }
    }

    document.querySelectorAll("[data-aurora-flow]").forEach((canvas) => {
      if (!(canvas instanceof HTMLCanvasElement) || canvas.dataset.auroraMounted === "true") return;
      canvas.dataset.auroraMounted = "true";
      const mounted = { canvas, unmount: mountAurora(canvas) };
      activeCanvases.add(mounted);
    });
  };

  initializeAurora();
  document.addEventListener("site:pagecontent-updated", initializeAurora);
})();
