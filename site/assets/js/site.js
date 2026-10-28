/* BRA 项目站点 — 导航、宠物互动、图表切换、返回顶部、代码复制 */
(function () {
  "use strict";

  var reduceMotion = window.matchMedia("(prefers-reduced-motion: reduce)").matches;
  var isEn = document.documentElement.lang.toLowerCase().indexOf("en") === 0;
  var imgBase = isEn ? "../assets/img/en/" : "assets/img/zh/";

  /* 页脚年份 */
  document.querySelectorAll("[data-year]").forEach(function (el) {
    el.textContent = String(new Date().getFullYear());
  });

  /* 当前页导航高亮 */
  var page = location.pathname.split("/").pop() || "index.html";
  document.querySelectorAll(".nav a[href]").forEach(function (a) {
    if (a.getAttribute("href") === page) a.setAttribute("aria-current", "page");
  });

  /* 头部滚动阴影 + 返回顶部显隐 */
  var header = document.querySelector(".site-header");
  var toTop = document.getElementById("to-top");
  function onScroll() {
    if (header) header.classList.toggle("is-scrolled", window.scrollY > 8);
    if (toTop) toTop.classList.toggle("is-visible", window.scrollY > 520);
  }
  onScroll();
  window.addEventListener("scroll", onScroll, { passive: true });

  /* 移动端菜单 */
  var toggle = document.querySelector(".nav-toggle");
  var nav = document.getElementById("site-nav");
  if (toggle && nav) {
    toggle.addEventListener("click", function () {
      var open = nav.classList.toggle("is-open");
      toggle.setAttribute("aria-expanded", open ? "true" : "false");
    });
    nav.addEventListener("click", function (e) {
      if (e.target.closest("a")) {
        nav.classList.remove("is-open");
        toggle.setAttribute("aria-expanded", "false");
      }
    });
    document.addEventListener("keydown", function (e) {
      if (e.key === "Escape" && nav.classList.contains("is-open")) {
        nav.classList.remove("is-open");
        toggle.setAttribute("aria-expanded", "false");
        toggle.focus();
      }
    });
    window.addEventListener("resize", function () {
      if (window.innerWidth > 1200 && nav.classList.contains("is-open")) {
        nav.classList.remove("is-open");
        toggle.setAttribute("aria-expanded", "false");
      }
    });
  }

  /* 滚动入场 */
  var reveals = document.querySelectorAll(".reveal");
  if (reduceMotion || !("IntersectionObserver" in window)) {
    reveals.forEach(function (el) { el.classList.add("is-visible"); });
  } else {
    var io = new IntersectionObserver(function (entries) {
      entries.forEach(function (entry) {
        if (entry.isIntersecting) {
          entry.target.classList.add("is-visible");
          io.unobserve(entry.target);
        }
      });
    }, { rootMargin: "0px 0px -8% 0px", threshold: 0.08 });
    reveals.forEach(function (el) { io.observe(el); });
  }

  /* 架构图切换 */
  var tabs = Array.prototype.slice.call(document.querySelectorAll(".tab[data-diagram]"));
  var panels = Array.prototype.slice.call(document.querySelectorAll(".diagram-panel"));
  tabs.forEach(function (tab) {
    tab.addEventListener("click", function () {
      tabs.forEach(function (t) { t.setAttribute("aria-selected", "false"); });
      tab.setAttribute("aria-selected", "true");
      panels.forEach(function (p) {
        p.classList.toggle("is-active", p.id === tab.dataset.diagram);
      });
    });
    tab.addEventListener("keydown", function (e) {
      var i = tabs.indexOf(tab);
      var next = e.key === "ArrowRight" ? i + 1 : e.key === "ArrowLeft" ? i - 1 : -1;
      if (next < 0) return;
      e.preventDefault();
      var target = tabs[(next + tabs.length) % tabs.length];
      target.focus();
      target.click();
    });
  });

  /* 阿守表情：点「换一种表情」切换；点图片放大 */
  var petNext = document.querySelector("[data-pet-next]");
  var petImg = document.querySelector(".pet-stage__pet img");
  var moods = Array.prototype.slice.call(document.querySelectorAll(".expression[data-mood]"));
  var moodStageImg = document.querySelector("[data-mood-img]");
  var moodName = document.querySelector("[data-mood-name]");
  var stage = document.querySelector(".mood-stage");

  var faces = isEn ? [
    ["smile", "Smile"], ["joy", "Joy"], ["cry", "Cry"], ["depressed", "Depressed"],
    ["terrified", "Terrified"], ["angry", "Angry"], ["coquettish", "Coquettish"], ["cute", "Cute"],
    ["pleasing", "Pleasing"], ["knocking", "Knocking"], ["facepalm", "Facepalm"], ["tongue", "Tongue Out"]
  ] : [
    ["smile", "笑"], ["joy", "欢乐"], ["cry", "哭"], ["depressed", "沮丧"],
    ["terrified", "惊恐"], ["angry", "愤怒"], ["coquettish", "撒娇"], ["cute", "卖萌"],
    ["pleasing", "讨好"], ["knocking", "敲打"], ["facepalm", "捂脸"], ["tongue", "吐舌头"]
  ];
  var faceIndex = 0;

  function expressionPath(name) {
    return imgBase + "pet/expressions/" + name + ".svg";
  }

  function pop(el) {
    if (!el || reduceMotion) return;
    el.classList.remove("is-popping");
    void el.offsetWidth;
    el.classList.add("is-popping");
    window.setTimeout(function () { el.classList.remove("is-popping"); }, 220);
  }

  function cyclePet() {
    if (!petImg) return;
    faceIndex = (faceIndex + 1) % faces.length;
    petImg.src = expressionPath(faces[faceIndex][0]);
    petImg.alt = isEn ? "Keeper's " + faces[faceIndex][1] + " expression" : "阿守的" + faces[faceIndex][1] + "表情";
    pop(petImg);
  }
  if (petNext) petNext.addEventListener("click", cyclePet);

  moods.forEach(function (btn) {
    btn.addEventListener("click", function () {
      moods.forEach(function (b) { b.setAttribute("aria-pressed", "false"); });
      btn.setAttribute("aria-pressed", "true");
      var name = btn.dataset.mood;
      var label = btn.dataset.moodName;
      if (moodStageImg) {
        moodStageImg.src = expressionPath(name);
        moodStageImg.alt = isEn ? "Keeper's " + label + " expression" : "阿守的" + label + "表情";
        pop(moodStageImg);
      }
      if (moodName) moodName.textContent = label;
      if (stage && !reduceMotion) {
        stage.animate(
          [{ transform: "scale(1)" }, { transform: "scale(0.97)" }, { transform: "scale(1)" }],
          { duration: 220, easing: "ease-out" }
        );
      }
    });
  });

  /* 返回顶部：按钮里只有阿守，没有文字 */
  if (toTop) {
    toTop.addEventListener("click", function () {
      window.scrollTo({ top: 0, behavior: reduceMotion ? "auto" : "smooth" });
    });
  }

  /* 代码块复制 */
  if (navigator.clipboard) {
    document.querySelectorAll(".doc pre").forEach(function (pre) {
      var btn = document.createElement("button");
      btn.className = "copy-btn";
      btn.type = "button";
      btn.textContent = isEn ? "Copy" : "复制";
      btn.addEventListener("click", function () {
        var code = pre.querySelector("code");
        var text = code ? code.textContent : pre.textContent;
        navigator.clipboard.writeText(text).then(function () {
          btn.textContent = isEn ? "Copied" : "已复制";
          window.setTimeout(function () { btn.textContent = isEn ? "Copy" : "复制"; }, 1400);
        }).catch(function () {
          btn.textContent = isEn ? "Copy failed" : "复制失败";
        });
      });
      pre.appendChild(btn);
    });
  }

  /* 图片点击放大 */
  var zoomables = document.querySelectorAll(".diagram-frame img, .pet-state img, .mood-stage img, .nf-pet, .doc img, .pet-stage__pet img, .expression img, .donate-card img");
  if (zoomables.length && "HTMLDialogElement" in window) {
    var dlg = document.createElement("dialog");
    dlg.className = "lightbox";
    dlg.setAttribute("aria-label", isEn ? "Image preview" : "图片预览");
    dlg.innerHTML = '<button class="lightbox__close" type="button" aria-label="' + (isEn ? "Close preview" : "关闭预览") + '">×</button><figure><img alt=""><figcaption></figcaption></figure>';
    document.body.appendChild(dlg);
    var dlgImg = dlg.querySelector("img");
    var dlgCap = dlg.querySelector("figcaption");
    var closeBtn = dlg.querySelector(".lightbox__close");

    function closeLightbox() {
      if (dlg.open) dlg.close();
      document.body.classList.remove("lightbox-open");
    }

    function labelFor(img) {
      if (img.alt) return img.alt;
      var btn = img.closest("button");
      if (btn) {
        if (btn.dataset.moodName) return isEn ? "Keeper's " + btn.dataset.moodName + " expression" : "阿守的" + btn.dataset.moodName + "表情";
        return btn.textContent.trim().replace(/\s+/g, " ");
      }
      return "";
    }

    function openLightbox(img) {
      var label = labelFor(img);
      dlgImg.src = img.currentSrc || img.src;
      dlgImg.alt = label;
      dlgCap.textContent = label;
      dlg.showModal();
      document.body.classList.add("lightbox-open");
      closeBtn.focus();
    }

    zoomables.forEach(function (img) {
      img.classList.add("zoomable");
      var nested = img.closest("button, a");
      if (!nested) {
        img.tabIndex = 0;
        img.setAttribute("role", "button");
        img.setAttribute("aria-label", "放大查看：" + (labelFor(img) || "图片"));
        img.addEventListener("keydown", function (e) {
          if (e.key === "Enter" || e.key === " ") {
            e.preventDefault();
            openLightbox(img);
          }
        });
      }
      img.addEventListener("click", function (e) {
        e.preventDefault();
        e.stopPropagation();
        openLightbox(img);
      });
    });

    closeBtn.addEventListener("click", closeLightbox);
    dlg.addEventListener("click", function (e) {
      if (e.target === dlg) closeLightbox();
    });
    dlg.addEventListener("close", function () {
      document.body.classList.remove("lightbox-open");
    });
  }

  /* 站点搜索 */
  var searchButtons = document.querySelectorAll("[data-search-open]");
  if (searchButtons.length && "HTMLDialogElement" in window) {
    var lang = isEn ? "en" : "zh";
    var dataBase = document.body.getAttribute("data-base");
    var base = dataBase !== null ? dataBase : (location.pathname.indexOf("/en/") !== -1 ? "../" : "");
    var t = isEn
      ? { placeholder: "Search the site…", close: "Close search", loading: "Loading search index…", empty: "Type to search…", none: "No matches", error: "Failed to load search index", count: "results", hint: "↑↓ select · Enter open · Esc close" }
      : { placeholder: "搜索站点…", close: "关闭搜索", loading: "正在加载搜索索引…", empty: "输入关键词搜索…", none: "没有找到匹配内容", error: "搜索索引加载失败", count: "条结果", hint: "↑↓ 选择 · Enter 打开 · Esc 关闭" };

    var sd = document.createElement("dialog");
    sd.className = "search-dialog";
    sd.setAttribute("aria-label", t.placeholder);
    sd.innerHTML = '<div class="search-panel"><div class="search-bar"><input type="search" autocomplete="off" placeholder="' + t.placeholder + '" aria-label="' + t.placeholder + '"><button class="search-close" type="button" aria-label="' + t.close + '">×</button></div><p class="search-status"></p><ul class="search-results"></ul><p class="search-hint"><span class="search-kbd">/</span> ' + t.hint + '</p></div>';
    document.body.appendChild(sd);
    var sInput = sd.querySelector("input");
    var sStatus = sd.querySelector(".search-status");
    var sResults = sd.querySelector(".search-results");
    var sClose = sd.querySelector(".search-close");
    var searchIndex = null;
    var searchError = false;
    var lastTrigger = null;
    var active = -1;
    var matches = [];

    function esc(s) {
      return String(s).replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;").replace(/"/g, "&quot;");
    }
    function escRe(s) { return s.replace(/[.*+?^${}()|[\]\\]/g, "\\$&"); }
    function highlight(s, terms) {
      var out = esc(s);
      var seen = {};
      terms.slice().sort(function (a, b) { return b.length - a.length; }).forEach(function (term) {
        if (!term || seen[term]) return;
        seen[term] = true;
        out = out.replace(new RegExp("(" + escRe(esc(term)) + ")", "gi"), "<mark>$1</mark>");
      });
      return out;
    }
    function termsFor(q) {
      q = q.toLowerCase().trim();
      var out = [q];
      q.split(/\s+/).forEach(function (w) { if (w) out.push(w); });
      if (/[\u4e00-\u9fff]/.test(q)) {
        for (var i = 0; i < q.length - 1; i++) {
          var bg = q.slice(i, i + 2);
          if (/[\u4e00-\u9fff]/.test(bg)) out.push(bg);
        }
      }
      return out.filter(function (v, i, a) { return v && a.indexOf(v) === i; });
    }
    function snippet(text, q, terms) {
      var lower = text.toLowerCase();
      var at = q ? lower.indexOf(q) : -1;
      if (at < 0) {
        for (var i = 0; i < terms.length; i++) {
          at = lower.indexOf(terms[i]);
          if (at >= 0) break;
        }
      }
      if (at < 0) at = 0;
      var start = Math.max(0, at - 45);
      var end = Math.min(text.length, at + 125);
      return highlight((start ? "…" : "") + text.slice(start, end) + (end < text.length ? "…" : ""), terms);
    }

    function loadIndex() {
      if (searchIndex || searchError) return Promise.resolve(searchIndex || []);
      sStatus.textContent = t.loading;
      return fetch(base + "search-index.json").then(function (r) {
        if (!r.ok) throw new Error("http " + r.status);
        return r.json();
      }).then(function (data) {
        searchIndex = data.filter(function (e) { return e.lang === lang; });
        return searchIndex;
      }).catch(function () {
        searchError = true;
        sStatus.textContent = t.error;
        return [];
      });
    }

    function render() {
      var q = sInput.value.trim();
      active = -1;
      if (!q) {
        matches = [];
        sResults.innerHTML = "";
        sStatus.textContent = t.empty;
        return;
      }
      var terms = termsFor(q);
      var lower = q.toLowerCase();
      var scored = (searchIndex || []).map(function (e) {
        var title = (e.title + " " + e.page).toLowerCase();
        var text = e.text.toLowerCase();
        var score = 0;
        if (title.indexOf(lower) >= 0) score += 20;
        if (text.indexOf(lower) >= 0) score += 10;
        terms.forEach(function (term) {
          if (title.indexOf(term) >= 0) score += 4;
          if (text.indexOf(term) >= 0) score += 1;
        });
        return { e: e, score: score };
      }).filter(function (x) { return x.score > 0; }).sort(function (a, b) { return b.score - a.score; }).slice(0, 20);
      matches = scored.map(function (x) { return x.e; });
      if (!matches.length) {
        sResults.innerHTML = "";
        sStatus.textContent = t.none;
        return;
      }
      sStatus.textContent = matches.length + " " + t.count;
      sResults.innerHTML = matches.map(function (e, i) {
        return '<li class="search-result"><a href="' + base + esc(e.url) + '" data-i="' + i + '"><span class="sr-page">' + esc(e.page) + '</span><span class="sr-title">' + highlight(e.title, terms) + '</span><span class="sr-snippet">' + snippet(e.text, lower, terms) + '</span></a></li>';
      }).join("");
    }

    function setActive(i) {
      var links = sResults.querySelectorAll("a");
      if (!links.length) return;
      active = (i + links.length) % links.length;
      links.forEach(function (a, n) { a.classList.toggle("is-active", n === active); });
      links[active].scrollIntoView({ block: "nearest" });
    }

    function openSearch() {
      sd.showModal();
      document.body.classList.add("search-open");
      sInput.focus();
      if (!searchIndex && !searchError) loadIndex().then(render);
      else render();
    }
    function closeSearch() {
      if (sd.open) sd.close();
      document.body.classList.remove("search-open");
      if (lastTrigger) lastTrigger.focus();
    }

    searchButtons.forEach(function (b) {
      b.addEventListener("click", function () { lastTrigger = b; openSearch(); });
    });
    sClose.addEventListener("click", closeSearch);
    sd.addEventListener("click", function (e) { if (e.target === sd) closeSearch(); });
    sd.addEventListener("keydown", function (e) {
      if (e.key === "Escape") { e.preventDefault(); closeSearch(); }
    });
    sd.addEventListener("close", function () {
      document.body.classList.remove("search-open");
      if (lastTrigger) lastTrigger.focus();
    });
    sInput.addEventListener("input", function () { loadIndex().then(render); });
    sInput.addEventListener("keydown", function (e) {
      if (e.key === "ArrowDown") { e.preventDefault(); setActive(active + 1); }
      else if (e.key === "ArrowUp") { e.preventDefault(); setActive(active - 1); }
      else if (e.key === "Enter") {
        var link = sResults.querySelector(active >= 0 ? 'a[data-i="' + active + '"]' : "a");
        if (link) { e.preventDefault(); link.click(); }
      }
    });
    document.addEventListener("keydown", function (e) {
      var tag = (e.target && e.target.tagName || "").toLowerCase();
      var typing = tag === "input" || tag === "textarea" || (e.target && e.target.isContentEditable);
      if (typing) return;
      if (e.key === "/" || ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === "k")) {
        e.preventDefault();
        openSearch();
      }
    });
  }
})();
