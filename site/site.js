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
  if (os !== "windows") {
    Array.prototype.forEach.call(document.querySelectorAll("a[data-dl-main]"), function (a) {
      var src = document.querySelector("a[data-href-" + os + "]");
      if (!src) return;
      var name = src.getAttribute("data-name-" + os), kind = src.getAttribute("data-kind-" + os);
      a.setAttribute("href", src.getAttribute("data-href-" + os));
      a.setAttribute("data-os", os);
      a.setAttribute("data-os-name", name);
      a.setAttribute("data-os-kind", kind);
      a.setAttribute("aria-label", "Download Sigil for " + name + " (" + kind + ")");
      if (a.hasAttribute("data-href-" + os)) a.textContent = "Download for " + name;
    });
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
