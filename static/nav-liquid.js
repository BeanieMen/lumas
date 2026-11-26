(() => {
  const activeLayers = new Map();
  const reducedMotion = window.matchMedia("(prefers-reduced-motion: reduce)");

  const vertexSource = `
    attribute vec2 a_position;
    varying vec2 v_uv;

    void main() {
      v_uv = a_position * 0.5 + 0.5;
      gl_Position = vec4(a_position, 0.0, 1.0);
    }
  `;

  const fragmentSource = `
    precision mediump float;

    uniform vec2 u_resolution;
    uniform float u_time;
    varying vec2 v_uv;

    float hash(vec2 p) {
      return fract(sin(dot(p, vec2(127.1, 311.7))) * 43758.5453123);
    }

    float noise(vec2 p) {
      vec2 i = floor(p);
      vec2 f = fract(p);
      f = f * f * (3.0 - 2.0 * f);
      return mix(
        mix(hash(i), hash(i + vec2(1.0, 0.0)), f.x),
        mix(hash(i + vec2(0.0, 1.0)), hash(i + vec2(1.0, 1.0)), f.x),
        f.y
      );
    }

    float flow(vec2 p) {
      float value = 0.0;
      float amplitude = 0.5;
      for (int i = 0; i < 4; i++) {
        value += amplitude * noise(p);
        p = p * 2.03 + vec2(13.7, 9.2);
        amplitude *= 0.5;
      }
      return value;
    }

    void main() {
      vec2 uv = v_uv;
      float time = u_time * 0.12;
      float aspect = u_resolution.x / u_resolution.y;
      vec2 p = uv * vec2(aspect, 1.0) * 2.2;
      float warpX = flow(p + vec2(time, -time * 0.35));
      float warpY = flow(p + vec2(7.1 - time * 0.4, 3.6 + time * 0.25));
      float liquid = flow(p + vec2(warpX, warpY) * 1.8 + vec2(-time, time * 0.3));
      float current = liquid + 0.1 * sin(p.x * 2.5 + liquid * 7.0 + time * 2.0);

      vec3 deep = vec3(0.016, 0.02, 0.043);
      vec3 mid = vec3(0.075, 0.302, 0.576);
      vec3 highlight = vec3(0.549, 0.925, 1.0);
      vec3 color = mix(deep, mid, smoothstep(0.34, 0.65, current));
      color = mix(color, highlight, smoothstep(0.69, 0.84, current) * 0.78);

      float edgeGlow = smoothstep(0.35, 0.8, flow(p * 0.7 + vec2(time * 0.45, 2.0)));
      color += vec3(0.015, 0.05, 0.1) * edgeGlow;
      gl_FragColor = vec4(color, 0.96);
    }
  `;

  const compileShader = (gl, type, source) => {
    const shader = gl.createShader(type);
    if (!shader) {
      console.error("Could not create the navigation liquid shader.");
      return null;
    }

    gl.shaderSource(shader, source);
    gl.compileShader(shader);
    if (!gl.getShaderParameter(shader, gl.COMPILE_STATUS)) {
      console.error("Navigation liquid shader compilation failed:", gl.getShaderInfoLog(shader));
      gl.deleteShader(shader);
      return null;
    }

    return shader;
  };

  const initializeLayer = (container) => {
    const canvas = container.querySelector(".nav-liquid-canvas");
    if (!canvas || activeLayers.has(container)) return;

    const gl = canvas.getContext("webgl", { alpha: true, antialias: false });
    if (!gl) {
      console.warn("WebGL is unavailable; using the navigation liquid gradient fallback.");
      return;
    }

    const vertexShader = compileShader(gl, gl.VERTEX_SHADER, vertexSource);
    const fragmentShader = compileShader(gl, gl.FRAGMENT_SHADER, fragmentSource);
    if (!vertexShader || !fragmentShader) return;

    const program = gl.createProgram();
    if (!program) {
      console.error("Could not create the navigation liquid shader program.");
      return;
    }

    gl.attachShader(program, vertexShader);
    gl.attachShader(program, fragmentShader);
    gl.linkProgram(program);
    gl.deleteShader(vertexShader);
    gl.deleteShader(fragmentShader);

    if (!gl.getProgramParameter(program, gl.LINK_STATUS)) {
      console.error("Navigation liquid shader linking failed:", gl.getProgramInfoLog(program));
      gl.deleteProgram(program);
      return;
    }

    const positionLocation = gl.getAttribLocation(program, "a_position");
    const resolutionLocation = gl.getUniformLocation(program, "u_resolution");
    const timeLocation = gl.getUniformLocation(program, "u_time");
    const buffer = gl.createBuffer();
    if (positionLocation < 0 || !resolutionLocation || !timeLocation || !buffer) {
      console.error("Could not initialize navigation liquid shader inputs.");
      gl.deleteProgram(program);
      if (buffer) gl.deleteBuffer(buffer);
      return;
    }

    gl.bindBuffer(gl.ARRAY_BUFFER, buffer);
    gl.bufferData(gl.ARRAY_BUFFER, new Float32Array([
      -1, -1, 1, -1, -1, 1,
      -1, 1, 1, -1, 1, 1,
    ]), gl.STATIC_DRAW);
    gl.useProgram(program);
    gl.enableVertexAttribArray(positionLocation);
    gl.vertexAttribPointer(positionLocation, 2, gl.FLOAT, false, 0, 0);

    const resize = () => {
      const bounds = container.getBoundingClientRect();
      const pixelRatio = Math.min(window.devicePixelRatio || 1, 1.5);
      const width = Math.max(1, Math.round(bounds.width * pixelRatio));
      const height = Math.max(1, Math.round(bounds.height * pixelRatio));
      if (canvas.width === width && canvas.height === height) return;

      canvas.width = width;
      canvas.height = height;
      gl.viewport(0, 0, width, height);
      gl.uniform2f(resolutionLocation, width, height);
      if (reducedMotion.matches) {
        gl.uniform1f(timeLocation, 0);
        gl.drawArrays(gl.TRIANGLES, 0, 6);
      }
    };

    let frame = 0;
    const draw = (timestamp) => {
      if (!document.hidden && !reducedMotion.matches) {
        gl.uniform1f(timeLocation, timestamp / 1000);
        gl.drawArrays(gl.TRIANGLES, 0, 6);
      }
      frame = window.requestAnimationFrame(draw);
    };

    resize();
    if (reducedMotion.matches) {
      gl.uniform1f(timeLocation, 0);
      gl.drawArrays(gl.TRIANGLES, 0, 6);
    } else {
      frame = window.requestAnimationFrame(draw);
    }

    const resizeObserver = new ResizeObserver(resize);
    resizeObserver.observe(container);

    const cleanup = () => {
      window.cancelAnimationFrame(frame);
      resizeObserver.disconnect();
      gl.deleteBuffer(buffer);
      gl.deleteProgram(program);
      activeLayers.delete(container);
    };
    activeLayers.set(container, cleanup);
  };

  const syncLayers = () => {
    for (const [container, cleanup] of activeLayers) {
      if (!container.isConnected) cleanup();
    }
    document.querySelectorAll(".nav").forEach(initializeLayer);
  };

  syncLayers();
  new MutationObserver(syncLayers).observe(document.documentElement, {
    childList: true,
    subtree: true,
  });

  reducedMotion.addEventListener("change", () => {
    for (const cleanup of activeLayers.values()) cleanup();
    syncLayers();
  });
})();
