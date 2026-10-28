/* BRA 项目站点 — 导航、宠物互动、图表切换、返回顶部、代码复制 */
(function () {
  "use strict";

  var reduceMotion = window.matchMedia("(prefers-reduced-motion: reduce)").matches;

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
      if (window.innerWidth > 960 && nav.classList.contains("is-open")) {
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

  var faces = [
    ["smile", "笑"], ["joy", "欢乐"], ["cry", "哭"], ["depressed", "沮丧"],
    ["terrified", "惊恐"], ["angry", "愤怒"], ["coquettish", "撒娇"], ["cute", "卖萌"],
    ["pleasing", "讨好"], ["knocking", "敲打"], ["facepalm", "捂脸"], ["tongue", "吐舌头"]
  ];
  var faceIndex = 0;

  function expressionPath(name) {
    return "assets/img/pet/expressions/" + name + ".svg";
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
    petImg.alt = "阿守的" + faces[faceIndex][1] + "表情";
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
        moodStageImg.alt = "阿守的" + label + "表情";
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
      btn.textContent = "复制";
      btn.addEventListener("click", function () {
        var code = pre.querySelector("code");
        var text = code ? code.textContent : pre.textContent;
        navigator.clipboard.writeText(text).then(function () {
          btn.textContent = "已复制";
          window.setTimeout(function () { btn.textContent = "复制"; }, 1400);
        }).catch(function () {
          btn.textContent = "复制失败";
        });
      });
      pre.appendChild(btn);
    });
  }

  /* 图片点击放大 */
  var zoomables = document.querySelectorAll(".diagram-frame img, .pet-state img, .mood-stage img, .nf-pet, .doc img, .pet-stage__pet img, .expression img");
  if (zoomables.length && "HTMLDialogElement" in window) {
    var dlg = document.createElement("dialog");
    dlg.className = "lightbox";
    dlg.setAttribute("aria-label", "图片预览");
    dlg.innerHTML = '<button class="lightbox__close" type="button" aria-label="关闭预览">×</button><figure><img alt=""><figcaption></figcaption></figure>';
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
        if (btn.dataset.moodName) return "阿守的" + btn.dataset.moodName + "表情";
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
})();
