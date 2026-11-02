// In the page's own world: before a site's passkey request, Herdr Autofill gets the vault ready to
// answer it (after Touch ID). The site's request then goes on as it would have. Passkey autofill on
// page load (conditional mediation) is left alone, so a page opening never asks for Touch ID.
(() => {
  const credentials = navigator.credentials;
  if (!credentials || credentials.__herdr) return;
  const original = { get: credentials.get.bind(credentials), create: credentials.create.bind(credentials) };
  const WAIT = 100_000;
  let next = 0;

  // Strings only across the two worlds; an object in detail doesn't cross.
  function askVault(mode, rpId) {
    return new Promise((resolve) => {
      const id = `${Date.now()}-${++next}`;
      const done = (event) => {
        let answer;
        try { answer = JSON.parse(event.detail); } catch { return; }
        if (answer.id !== id) return;
        document.removeEventListener("herdr-passkey-ready", done);
        resolve(answer);
      };
      document.addEventListener("herdr-passkey-ready", done);
      document.dispatchEvent(new CustomEvent("herdr-passkey", { detail: JSON.stringify({ id, mode, rpId }) }));
      setTimeout(() => resolve({}), WAIT);
    });
  }

  credentials.get = async (options) => {
    if (options?.publicKey && options.mediation !== "conditional") await askVault("get", options.publicKey.rpId ?? "");
    return original.get(options);
  };
  credentials.create = async (options) => {
    if (options?.publicKey) await askVault("create", options.publicKey.rp?.id ?? "");
    return original.create(options);
  };
  Object.defineProperty(credentials, "__herdr", { value: true });
})();
