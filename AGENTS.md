# Page conventions

- New page templates should extend `templates/base.html` and put page content inside its `body` block. The base template owns the shared `#page-content` container and background music player; do not add another player or page-specific music startup logic.
- Internal page links inside `#page-content` use `static/site-navigation.js` for same-document navigation, keeping the music playing. Keep normal browser navigation for external destinations, downloads, and authentication routes.
- If a page needs its own stylesheet, give its link element `id="page-styles"`. Client-side navigation waits for that stylesheet before replacing page content, then refreshes AOS animations.
- Keep page assets referenced with root-relative URLs so they resolve after client-side navigation.
- When adding a page, verify navigating to it and back updates its content and stylesheet while preserving the shared audio player; check that page images, icons, and AOS animations load.

## Lumas Heatwave font
- The site uses a custom full-color bitmap font, "Lumas Heatwave", loaded via @font-face in the global CSS from the Hack Club CDN. Those CDN URLs are canonical: never re-host, convert, or swap in a different font.
- The font only has A-Z, a-z, 0-9 and space. Punctuation and any other character falls back to Georgia, serif — that's expected.
- Apply the .lumas-font class only to the logo text, hero heading and section headings (h1, h2). Never paragraphs or long text.
- Never set color, text-shadow, filter, gradient or background-clip on it. The colors live inside the font; CSS color has no effect.
- Never set font-weight: bold or font-style: italic on it. The browser will fake it and smear the painted letters.
- Minimum size 28px; 40px-120px looks best. Dark backgrounds only (navy or near-black).
- If the font fails to load or letters render flat white/uncolored, flag it instead of silently substituting another font.