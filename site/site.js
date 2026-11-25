// Sigil site: the Features menu, the download for the visitor's system and the download event. The page works without this file.
(function () {
  document.documentElement.classList.add("js");

  // Vercel Web Analytics custom event. window.va is a queue stub in the page head, so this is
  // safe when the analytics script is absent (local preview).
  document.addEventListener("click", function (e) {
    var link = e.target.closest && e.target.closest("a[data-download]");
    if (!link) return;
    try {
      if (typeof window.va === "function") {
        window.va("event", { name: "download", data: {
          position: link.getAttribute("data-download"),
          os: link.getAttribute("data-os") || "windows"
        } });
      }
    } catch (_) { /* analytics must never block the download */ }
  });

  // Offer the visitor's own system first. The HTML defaults to Windows and lists all three, so
  // this only reorders. Phones and anything unknown keep the Windows default.
  var os = (function () {
    var ua = navigator.userAgent || "";
    var p = (navigator.userAgentData && navigator.userAgentData.platform) || navigator.platform || "";
    if (/android|iphone|ipad|ipod/i.test(ua)) return "windows";
    var s = p || ua;
    if (/mac/i.test(s)) return "macos";
    if (/linux|x11|cros/i.test(s)) return "linux";
    return "windows";
  })();
  var ORDER = ["windows", "linux", "macos"];
  function info(a) {
    return { os: a.getAttribute("data-os"), name: a.getAttribute("data-os-name"),
             kind: a.getAttribute("data-os-kind"), href: a.getAttribute("href") };
  }
  function apply(a, d) {
    a.setAttribute("href", d.href);
    a.setAttribute("data-os", d.os);
    a.setAttribute("data-os-name", d.name);
    a.setAttribute("data-os-kind", d.kind);
    a.setAttribute("aria-label", "Download Sigil for " + d.name + " (" + d.kind + ")");
  }
  if (os !== "windows") {
    var found = null;
    Array.prototype.forEach.call(document.querySelectorAll("[data-dl]"), function (group) {
      var links = Array.prototype.slice.call(group.querySelectorAll("a[data-os]"));
      var all = links.map(info).sort(function (x, y) { return ORDER.indexOf(x.os) - ORDER.indexOf(y.os); });
      var mine = all.filter(function (d) { return d.os === os; })[0];
      if (!mine) return;
      found = mine;
      var rest = all.filter(function (d) { return d !== mine; });
      apply(links[0], mine);
      links[0].textContent = "Download for " + mine.name;
      rest.forEach(function (d, i) { apply(links[i + 1], d); links[i + 1].textContent = d.name; });
    });
    var nav = document.querySelector(".bar a[data-dl-main]");
    if (nav && found) apply(nav, found);
  }

  var menu = document.querySelector("[data-menu]");
  if (!menu) return;
  var trigger = menu.querySelector(".menu__trigger");
  var panel = menu.querySelector(".menu__panel");
  var links = function () { return Array.prototype.slice.call(panel.querySelectorAll("a")); };

  function open(focusFirst) {
    menu.classList.add("is-open");
    trigger.setAttribute("aria-expanded", "true");
    if (focusFirst) links()[0].focus();
  }
  function close(returnFocus) {
    menu.classList.remove("is-open");
    trigger.setAttribute("aria-expanded", "false");
    if (returnFocus) trigger.focus();
  }
  function isOpen() { return menu.classList.contains("is-open"); }

  trigger.addEventListener("click", function () { isOpen() ? close(false) : open(false); });
  trigger.addEventListener("keydown", function (e) {
    if (e.key === "ArrowDown") { e.preventDefault(); e.stopPropagation(); open(true); }
  });
  menu.addEventListener("keydown", function (e) {
    if (e.key === "Escape" && isOpen()) { e.preventDefault(); close(true); return; }
    if (!isOpen() || (e.key !== "ArrowDown" && e.key !== "ArrowUp" && e.key !== "Home" && e.key !== "End")) return;
    var items = links();
    var i = items.indexOf(document.activeElement);
    if (i === -1 && e.key !== "ArrowDown") return;
    e.preventDefault();
    if (e.key === "Home") i = 0;
    else if (e.key === "End") i = items.length - 1;
    else i = (i + (e.key === "ArrowDown" ? 1 : -1) + items.length) % items.length;
    items[i].focus();
  });
  menu.addEventListener("focusout", function (e) {
    if (isOpen() && !menu.contains(e.relatedTarget)) close(false);
  });
  document.addEventListener("click", function (e) {
    if (isOpen() && !menu.contains(e.target)) close(false);
  });
  panel.addEventListener("click", function (e) {
    if (e.target.closest("a")) close(false);
  });
})();
