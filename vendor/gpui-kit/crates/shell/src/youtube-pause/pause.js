// A YouTube page that comes back after a restart shouldn't start playing by itself.
// Pause any video that starts before the person has touched the page; a real click
// or keypress hands control back, including for a video they pick afterwards.
(() => {
  if (window.__herdrYoutubePause) return;
  window.__herdrYoutubePause = true;

  let touched = false;
  const touch = () => { touched = true; };
  addEventListener("pointerdown", touch, { capture: true, once: true });
  addEventListener("keydown", touch, { capture: true, once: true });

  const pause = () => {
    if (touched) return;
    for (const video of document.querySelectorAll("video")) {
      if (!video.paused) video.pause();
    }
  };

  // Players mount late and may autoplay again after their first frame.
  const timer = setInterval(pause, 250);
  setTimeout(() => clearInterval(timer), 15000);
  pause();
})();
