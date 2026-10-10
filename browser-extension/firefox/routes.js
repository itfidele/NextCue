(function (root) {
  "use strict";

  function subdomainOf(host, domain) {
    return host === domain || host.endsWith(`.${domain}`);
  }

  function providerForUrl(value) {
    let url;
    try {
      url = new URL(value);
    } catch (_) {
      return null;
    }
    if (url.protocol !== "https:" || url.username || url.password || (url.port && url.port !== "443")) {
      return null;
    }
    const host = url.hostname;
    const path = url.pathname.replace(/\/$/, "");

    if (host === "meet.google.com" && (/^\/[a-z]{3}-[a-z]{4}-[a-z]{3}$/.test(path) || /^\/lookup\/[A-Za-z0-9_-]{1,128}$/.test(path))) {
      return "Google Meet";
    }
    if ((subdomainOf(host, "zoom.us") || subdomainOf(host, "zoomgov.com")) && (/^\/wc\/(?:join\/)?\d{6,15}(?:\/(?:join|start))?$/.test(path) || /^\/j\/\d{6,15}$/.test(path))) {
      return "Zoom";
    }
    if (["teams.microsoft.com", "teams.live.com", "teams.cloud.microsoft"].includes(host)) {
      if (/^\/(?:v2\/)?l\/meetup-join\/[^/]{1,1024}$/.test(path) || /^\/meet\/\d{6,30}$/.test(path) || /^\/meet\/[A-Za-z0-9_-]{6,128}$/.test(path)) {
        return "Microsoft Teams";
      }
      // Some Teams web clients keep the meeting route in the fragment. The
      // fragment is examined locally and is never sent to the desktop app.
      if (/^#\/?(?:v2\/)?l\/meetup-join\/[^?]{1,1024}(?:\?.*)?$/.test(url.hash)) {
        return "Microsoft Teams";
      }
    }
    if (host.endsWith(".webex.com") && (/^\/meet\/[A-Za-z0-9_.-]{1,128}$/.test(path) || /^\/(?:wc|join)\/[A-Za-z0-9_-]{1,128}$/.test(path) || /^\/(?:[^/]+\/)?j\.php$/.test(path))) {
      return "Webex";
    }
    if (host === "meet.jit.si" && /^\/[A-Za-z0-9_-]{1,128}$/.test(path) && !["/static", "/config", "/about", "/help"].includes(path)) {
      return "Jitsi";
    }
    if (host === "whereby.com" && /^\/[A-Za-z0-9_-]{1,128}$/.test(path) && !["/login", "/signup", "/pricing", "/information", "/business", "/terms", "/privacy"].includes(path)) {
      return "Whereby";
    }
    return null;
  }

  function parseConnection(value) {
    const parsed = typeof value === "string" ? JSON.parse(value) : value;
    if (!parsed || typeof parsed !== "object" || !Number.isInteger(parsed.port) || parsed.port < 1024 || parsed.port > 65535 || typeof parsed.token !== "string" || !/^[A-Za-z0-9_-]{43}$/.test(parsed.token)) {
      throw new Error("Paste a valid connection code copied from NextCue Settings.");
    }
    // Ignore display metadata from the trusted desktop UI; no host or URL in
    // the pasted JSON can change the hardcoded loopback destination.
    return { port: parsed.port, token: parsed.token };
  }

  const api = { providerForUrl, parseConnection };
  root.NextCueRoutes = api;
  if (typeof module === "object" && module.exports) module.exports = api;
})(globalThis);
