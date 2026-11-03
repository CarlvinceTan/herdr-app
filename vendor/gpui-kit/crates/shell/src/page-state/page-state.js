// A page in a pane comes back where it was after Herdr restarts: its scroll
// position and its video/audio position. The page's own localStorage persists
// in the browser profile, so this survives a quit as well as a reload.
(() => {
  if (window.__herdrPageState) return;
  window.__herdrPageState = true;

  const saveKey = () => `herdr.page-state:${location.href}`;
  const MAX_AGE = 7 * 24 * 60 * 60 * 1000;

  function media() {
    return [...document.querySelectorAll("video,audio")].slice(0, 5);
  }

  function save() {
    const state = {
      at: Date.now(),
      x: window.scrollX,
      y: window.scrollY,
      media: media().map((el) => ({ t: el.currentTime, paused: el.paused, muted: el.muted, rate: el.playbackRate })),
    };
    try {
      localStorage.setItem(saveKey(), JSON.stringify(state));
      // Keep only recent URLs: the profile lives as long as Herdr does.
      const keys = Object.keys(localStorage).filter((key) => key.startsWith("herdr.page-state:"));
      if (keys.length > 80) {
        keys.map((key) => [key, Number(JSON.parse(localStorage.getItem(key) || "{}").at) || 0])
          .sort((a, b) => a[1] - b[1])
          .slice(0, keys.length - 80)
          .forEach(([key]) => localStorage.removeItem(key));
      }
    } catch {}
  }

  setInterval(save, 1000);
  addEventListener("pagehide", save);
  document.addEventListener("visibilitychange", () => { if (document.hidden) save(); });

  let state = null;
  try { state = JSON.parse(localStorage.getItem(saveKey()) || "null"); } catch {}
  if (!state || Date.now() - (state.at ?? 0) > MAX_AGE) return;

  function restoreScroll() {
    try { window.scrollTo(state.x ?? 0, state.y ?? 0); } catch {}
  }
  for (const delay of [0, 300, 1200, 2500]) setTimeout(restoreScroll, delay);

  function restoreMedia() {
    const els = media();
    if (!els.length) return false;
    els.forEach((el, i) => {
      const s = state.media?.[i];
      if (!s || !Number.isFinite(s.t)) return;
      const apply = () => {
        try {
          if (s.t > 0.5) el.currentTime = s.t;
          el.muted = Boolean(s.muted);
          if (s.rate) el.playbackRate = s.rate;
          if (!s.paused) el.play().catch(() => {});
        } catch {}
      };
      if (el.readyState >= 1) apply();
      else el.addEventListener("loadedmetadata", apply, { once: true });
    });
    return true;
  }

  if (!restoreMedia()) {
    // A player (YouTube) mounts its video after the page loads.
    const observer = new MutationObserver(() => { if (restoreMedia()) observer.disconnect(); });
    observer.observe(document.documentElement, { childList: true, subtree: true });
    setTimeout(() => observer.disconnect(), 10000);
  }
})();
