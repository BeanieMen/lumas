# Lumas

## Hover audio

The delegated hover-audio module is mounted in `templates/base.html`, so it is available on every page. Add `data-audio="1"` through `data-audio="16"` to any element to play that sprite clip on hover. Use `data-audio-volume="0.5"` for an element-specific volume from 0 to 1 (default 1), and `data-audio-event="click"` to trigger on click instead.

The MP3 sprite has 16 slots of 2.4 seconds each. Clip N starts at `(N - 1) × 2.4` seconds: clips 1–16 start at 0, 2.4, 4.8, 7.2, 9.6, 12, 14.4, 16.8, 19.2, 21.6, 24, 26.4, 28.8, 31.2, 33.6, and 36 seconds. Playback is limited to the selected 2.4-second slot.

Set the `AUDIO_URL` constant near the top of `static/hover-audio.js` to the CDN URL for the sprite MP3. The feature remains silent until that URL is configured and the visitor makes a user gesture.

Mute or unmute from client-side JavaScript with `window.hoverAudio.setMuted(true)` or `window.hoverAudio.setMuted(false)`. Check the current setting with `window.hoverAudio.isMuted()`; it is saved in local storage.

The home page includes test examples: the Proposal navigation link plays clip 1, the Test hover sound button plays clip 2, and the Example audio card plays clip 3.