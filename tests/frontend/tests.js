// Smoke tests for the real UI. Runs after app.js; results are written to #out.
(async () => {
  // The app's markup replaces the runner page, so the results box is created here.
  const out = document.getElementById("out") || document.body.appendChild(Object.assign(document.createElement("pre"), { id: "out" }));
  const results = [];
  const sleep = ms => new Promise(r => setTimeout(r, ms));
  async function until(cond, ms = 4000) {
    const start = Date.now();
    while (Date.now() - start < ms) {
      try { if (cond()) return true; } catch (e) { /* not ready */ }
      await sleep(50);
    }
    return false;
  }
  function check(name, ok, detail = "") {
    results.push(`${ok ? "PASS" : "FAIL"}  ${name}${detail ? "  (" + detail + ")" : ""}`);
  }
  const $ = s => document.querySelector(s);
  const $$ = s => [...document.querySelectorAll(s)];

  try {
    // 1. Trending renders; hostile title stays text, not markup
    check("trending cards render", await until(() => $$("#recent .card").length === 1));
    check("hostile title is not parsed as HTML", $$("#recent img").length === 1 && !window.__pwned,
      `imgs=${$$("#recent img").length}`);
    check("hostile title shows as literal text", ($("#recent .card-title")?.textContent || "").includes("<img"));

    // 2. Search via Enter key
    $("#q").value = "sho";
    $("#q").dispatchEvent(new KeyboardEvent("keydown", { key: "Enter" }));
    check("search returns results", await until(() => $$("#results .card").length === 1));

    // 3. Open detail; episodes listed
    $("#results .card").click();
    check("detail shows episodes", await until(() => $$(".ep-btn").length === 3));
    check("detail title rendered", ($(".detail-title")?.textContent || "") === "Show One");

    // 4. Episode opens the player with quality buttons
    $$(".ep-btn")[0].click();
    check("player opens with qualities", await until(() => $("#vp") && $$(".qbtn").length === 2, 6000));

    // 5. Quality button switches the active source
    $$(".qbtn")[1].click();
    check("quality switch marks active button", await until(() => $$(".qbtn")[1].classList.contains("bp")));

    // 6. Header navigation to library, filter buttons present
    $('[data-act="showLibrary"]').click();
    check("library filters render", await until(() => $("#lib-filters") && $$("#lib-filters .btn").length === 6, 6000));

    // 7. Event delegation: no inline handlers left in markup
    check("no inline handlers in markup", !$$("[onclick],[onchange],[oninput],[onerror]").length);

    // 8. CSP: nothing blocked by policy during the run
    check("no CSP violations", window.__violations.length === 0, window.__violations.join("; "));
    check("bridge called for trending", window.__calls.includes("get_recently_updated"));
  } catch (e) {
    results.push(`FAIL  test run threw: ${e && e.message}`);
  }

  const failed = results.filter(r => r.startsWith("FAIL")).length;
  out.textContent = results.join("\n") + `\n\n${failed ? failed + " failed" : "all passed"}`;
  window.__done = true;
})();
