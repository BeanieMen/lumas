# Page conventions

- New page templates should extend `templates/base.html` and put page content inside its `body` block. The base template owns the shared `#page-content` container and background music player; do not add another player or page-specific music startup logic.
- Internal page links inside `#page-content` use `static/site-navigation.js` for same-document navigation, keeping the music playing. Keep normal browser navigation for external destinations, downloads, and authentication routes.
- If a page needs its own stylesheet, give its link element `id="page-styles"`. Client-side navigation waits for that stylesheet before replacing page content, then refreshes AOS animations.
- Keep page assets referenced with root-relative URLs so they resolve after client-side navigation.
- When adding a page, verify navigating to it and back updates its content and stylesheet while preserving the shared audio player; check that page images, icons, and AOS animations load.
