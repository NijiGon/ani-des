// Loads the real src/index.html with the Tauri mock and the test script, under the CSP from tauri.conf.json.
(async () => {
  const html = await (await fetch("../../src/index.html")).text();
  const conf = await (await fetch("../../src-tauri/tauri.conf.json")).json();
  const csp = conf.app.security.csp;
  const head = `<head><base href="../../src/"><meta http-equiv="Content-Security-Policy" content="${csp}"><script src="../tests/frontend/mock-tauri.js"></script>`;
  const page = html
    .replace("<head>", head)
    .replace("</body>", `<script src="../tests/frontend/tests.js"></script></body>`);
  document.open();
  document.write(page);
  document.close();
})();
