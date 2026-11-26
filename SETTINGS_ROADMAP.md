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

## Planned (priority order)
| id | Setting | Source | Ease | Notes |
|----|---------|--------|------|-------|
| 2 | Restore last queue on start | Metro | Medium | serialise queue into Prefs |
| 4 | Output device picker | BC | Medium | `cpal` lists devices |
| 6 | Lyrics: text size, alignment, auto-scroll, tap to seek | Metro, BC | Easy-Med | constants in lyrics drawing, `player.rs` |
| 7 | Reduce motion / reduce blur | BC | Medium | one flag gating cover + lyric blur first; helps weak GPUs |
| 8 | Auto-skip on error | Metro | Easy-Med | hook in `Player::fail` |
| 9 | Discord: activity type, status text, buttons | Metro | Easy | `discord.rs` already builds the activity; flesh out within reason (type, status display, 2 buttons, presence preview) |
| 10 | Listen Together: server URL, auto-approve joins/suggestions, sync volume | Metro | Easy-Med | partly in the party popover (`party.rs`) |
| 15 | Image cache size and clear | Metro, BC | Easy-Med | |
| 16 | Start minimized, autostart, media keys | desktop | Medium | platform specific |
| 17 | Hide home sections (recents, moods, top picks, Shorts), prefer music only | Metro, BC | Medium | filter the daemon's home feed |
| 18 | Show auto playlists (Liked, Downloaded, Top) | Metro | Easy | |
| 22 | Update checker + changelog | Metro | Medium | |
| 24 | Equalizer, Automix / smart fade, bit-perfect | BC | Hard | real DSP work |

## Design notes
- Tabs (7), text only with an accent underline: Appearance, Playback, Library (content + lyrics), Storage, Privacy, Accounts, System (shortcuts + reset). Search is a magnifier at the right end of the bar. Drill-down and sidebar layouts were tried in design/settings-prototype.html and rejected.
- Rows are declarative via `Page::row`; search filters rows by title/description across the searchable tabs.
- Every new preference is a field on `Opts` (serde default) and, if the engine needs it, applied in `App::new` and in `reset_opts`.
