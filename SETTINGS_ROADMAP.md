# Serafin settings roadmap

Settings ported or planned from Metrolist (Android) and BitChord. The page lives in `client/egui-app/src/settings.rs`.
Android-only options (Android Auto, Cast, audio offload, swipe gestures, navbar, app language, ...) are out of scope.
Number = id from the original ranking. Not wanted: 5 (start page), 21 (skip silence), 23 (AI lyrics translation), 25 (stats/Wrapped).

## Done
Mode, theme from artwork, accent, custom theme, pure black, row density, header tint, font, tray on close, audio quality,
normalize, crossfade, gapless, Endless, no duplicate songs, music videos + quality, seek/volume step, sleep timer, hide
explicit/video, artist page parts, language/country, proxy, romaji, lyrics provider order, private session, YouTube
history, remember searches, audio cache, Discord toggle, Last.fm, reset preferences.

## Done in the redesign pass
- Text-only tab bar (Appearance, Playback, Library, Storage, Privacy, Accounts, System) with expanding search; new components (stepper, picker pill, meter, swatch grid, mode preview tiles)
- 1 UI scale (80-150%, window minimum size follows it)
- 3 Remember shuffle and repeat
- 12 Loudness target (Quiet -19 / Balanced -14 default / Loud -11)
- 13 Sleep timer fade-out (stop-after-current-song already existed as "End of song")
- 14 Export / import preferences (serafin-preferences.json in the data folder)
- 20 Progressive seek
- Also done, not on the original list: scroll speed (Slow/Normal/Fast, Appearance > Display), crossfade overlap graph, explicit "E" badge on song rows, queue, now-playing and home cards (not yet in the command palette rows)
- Skipped: 11 add-to-playlist position (the client has no add-to-playlist call yet; add it with that feature), 19 repeat-all auto-load (Endless already does not load while repeat is on)

## Done in the port pass
- 2 Restore last queue on start
- 4 Output device picker (falls back to default if the device disappears)
- 6 Lyrics: text size, alignment, auto-scroll, tap to seek
- 7 Reduce motion (lyric blur and animations; cover backdrop blur not gated, the text ink colour depends on it)
- 8 Auto-skip on error (stops after one pass through the queue)
- 9 Discord: toggles for "Listen on YouTube Music" and "Get Serafin" buttons (activity otherwise unchanged)
- 10 Listen Together settings (sync volume dropped on purpose)
- 15 Image cache size and clear
- 16 Start minimized, autostart (Linux only); media keys already handled by `media.rs`
- 17 Hide home sections, music only
- 18 Show auto playlists
- 22 Update checker + changelog (GitHub releases of hireri/serafin)

## Planned
| id | Setting | Source | Ease | Notes |
|----|---------|--------|------|-------|
| 24 | Equalizer, Automix / smart fade, bit-perfect | BC | Hard | real DSP work |

## Design notes
- Tabs (7), text only with an accent underline: Appearance, Playback, Library (content + lyrics), Storage, Privacy, Accounts, System (shortcuts + reset). Search is a magnifier at the right end of the bar. Drill-down and sidebar layouts were tried in design/settings-prototype.html and rejected.
- Rows are declarative via `Page::row`; search filters rows by title/description across the searchable tabs.
- Every new preference is a field on `Opts` (serde default) and, if the engine needs it, applied in `App::new` and in `reset_opts`.
