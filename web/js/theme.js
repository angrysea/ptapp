// Appearance chosen on Profile (light, dark, or the device's setting when
// unset). A classic script loaded in <head>, so the page paints in the
// chosen colours from the start. Kept on the device, like the outbox.
try {
  const t = localStorage.getItem('pt.theme');
  if (t === 'light' || t === 'dark') document.documentElement.dataset.theme = t;
} catch { /* storage unavailable: follow the device */ }
