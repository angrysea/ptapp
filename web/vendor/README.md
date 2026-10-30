# Vendored libraries

Third-party scripts the PWA loads from its own origin (no build step, no CDN at runtime). mp4box and mp4-muxer are loaded only when a set video is converted (`web/js/encode.js`) and are precached by the service worker so conversion works offline. pdf.js is loaded only when a recipe is imported from a PDF (`web/js/views/recipes.js`, which reads the text on the device); it isn't precached, as importing needs a connection anyway.

| File | Package | Source | License | SHA-384 |
|---|---|---|---|---|
| `mp4box-0.5.2.all.min.js` | [mp4box](https://github.com/gpac/mp4box.js) 0.5.2 | `https://cdn.jsdelivr.net/npm/mp4box@0.5.2/dist/mp4box.all.min.js` | BSD-3-Clause | `9oK16t6TCKqCB5OpB/LwRkNa48prlRFQPTYw8tb9uj699JvQ1wjxIq7YO8Z4hn3a` |
| `mp4-muxer-5.1.3.js` | [mp4-muxer](https://github.com/Vanilagy/mp4-muxer) 5.1.3 | `https://cdn.jsdelivr.net/npm/mp4-muxer@5.1.3/build/mp4-muxer.js` | MIT | `SujebcgqCNlLMRSnVkLD+3eWOzsxobv30Bct+0lV+fc1EHHEuvcePwmDgMzpnptj` |
| `pdfjs-6.4.299/pdf.min.mjs` | [pdfjs-dist](https://github.com/mozilla/pdf.js) 6.4.299 | `https://cdn.jsdelivr.net/npm/pdfjs-dist@6.4.299/build/pdf.min.mjs` | Apache-2.0 | `QCvhu/UPp22U/TNLVbQNONL82ES2yY717GAiqP9VVu37Y7+m9VPKvadGx9XlFvcp` |
| `pdfjs-6.4.299/pdf.worker.min.mjs` | pdfjs-dist 6.4.299 (its worker) | `https://cdn.jsdelivr.net/npm/pdfjs-dist@6.4.299/build/pdf.worker.min.mjs` | Apache-2.0 | `NfW/OTMWezITv4oaALvfH7MFLwzPxEvpySmDpoALa1mtTY8MC200l0ZNFD+SOlrM` |

To verify: `openssl dgst -sha384 -binary <file> | openssl base64 -A`. To update, download the new published build, record its version and hash here, and rename the file (the version is in the name so the service worker cache can't serve a stale copy).
