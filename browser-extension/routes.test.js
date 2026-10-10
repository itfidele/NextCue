"use strict";
const test = require("node:test");
const assert = require("node:assert/strict");
const { providerForUrl, parseConnection } = require("./routes.js");

test("recognizes supported meeting routes without treating provider home pages as calls", () => {
  const cases = [
    ["https://meet.google.com/abc-defg-hij?authuser=1", "Google Meet"],
    ["https://meet.google.com/lookup/team-standup", "Google Meet"],
    ["https://us02web.zoom.us/wc/12345678901/join?pwd=secret", "Zoom"],
    ["https://zoom.us/wc/join/12345678901", "Zoom"],
    ["https://zoomgov.com/j/12345678901", "Zoom"],
    ["https://teams.microsoft.com/l/meetup-join/19%3ameeting%40thread.v2?context=private", "Microsoft Teams"],
    ["https://teams.cloud.microsoft/meet/1234567890123", "Microsoft Teams"],
    ["https://teams.live.com/v2/#/l/meetup-join/example", "Microsoft Teams"],
    ["https://company.webex.com/meet/person", "Webex"],
    ["https://company.webex.com/company/j.php?MTID=secret", "Webex"],
    ["https://meet.jit.si/TeamRoom", "Jitsi"],
    ["https://whereby.com/team-room", "Whereby"],
  ];
  for (const [url, provider] of cases) assert.equal(providerForUrl(url), provider, url);
  for (const url of ["https://meet.google.com/", "https://zoom.us/", "https://zoom.us/signin", "https://teams.microsoft.com/v2/", "https://company.webex.com/", "https://meet.jit.si/about", "https://whereby.com/pricing"]) {
    assert.equal(providerForUrl(url), null, url);
  }
});

test("rejects lookalike hosts, non-HTTPS URLs, unusual ports and embedded credentials", () => {
  for (const url of [
    "https://meet.google.com.evil.test/abc-defg-hij",
    "https://evilzoom.us/wc/12345678901/join",
    "https://company.webex.com.evil.test/meet/person",
    "https://meet.jit.si.evil.test/TeamRoom",
    "http://meet.google.com/abc-defg-hij",
    "https://meet.google.com:8443/abc-defg-hij",
    "https://name:secret@meet.google.com/abc-defg-hij",
    "file:///meet.google.com/abc-defg-hij",
    "javascript:alert(1)",
    "not a URL",
  ]) assert.equal(providerForUrl(url), null, url);
});

test("connection code accepts bounded ports and 256-bit tokens while ignoring supplied hosts", () => {
  const token = "a".repeat(43);
  assert.deepEqual(parseConnection(JSON.stringify({ port: 32123, token, host: "evil.test", available: true })), { port: 32123, token });
  for (const value of [null, {}, { port: 0, token }, { port: 1023, token }, { port: 65536, token }, { port: 32123.5, token }, { port: "32123", token }, { port: 32123, token: "short" }, { port: 32123, token: `${token}\n` }, { port: 32123, token: "!".repeat(43) }]) {
    assert.throws(() => parseConnection(value));
  }
});
