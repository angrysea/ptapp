# Vendored libraries

Third-party scripts the PWA loads from its own origin (no build step, no CDN at runtime). They are loaded only when a set video is converted (`web/js/encode.js`) and are precached by the service worker so conversion works offline.

| File | Package | Source | License | SHA-384 |
|---|---|---|---|---|
| `mp4box-0.5.2.all.min.js` | [mp4box](https://github.com/gpac/mp4box.js) 0.5.2 | `https://cdn.jsdelivr.net/npm/mp4box@0.5.2/dist/mp4box.all.min.js` | BSD-3-Clause | `9oK16t6TCKqCB5OpB/LwRkNa48prlRFQPTYw8tb9uj699JvQ1wjxIq7YO8Z4hn3a` |
| `mp4-muxer-5.1.3.js` | [mp4-muxer](https://github.com/Vanilagy/mp4-muxer) 5.1.3 | `https://cdn.jsdelivr.net/npm/mp4-muxer@5.1.3/build/mp4-muxer.js` | MIT | `SujebcgqCNlLMRSnVkLD+3eWOzsxobv30Bct+0lV+fc1EHHEuvcePwmDgMzpnptj` |

To verify: `openssl dgst -sha384 -binary <file> | openssl base64 -A`. To update, download the new published build, record its version and hash here, and rename the file (the version is in the name so the service worker cache can't serve a stale copy).
