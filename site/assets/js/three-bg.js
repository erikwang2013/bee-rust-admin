/* 首屏蜂巢背景：three.js 本地模块，无外部网络请求 */
import * as THREE from "./three.module.min.js";

const canvas = document.getElementById("hive-canvas");
if (canvas) init(canvas);

function init(canvas) {
  const reduceMotion = window.matchMedia("(prefers-reduced-motion: reduce)").matches;
  let renderer;
  try {
    renderer = new THREE.WebGLRenderer({
      canvas,
      alpha: true,
      antialias: true,
      powerPreference: "high-performance"
    });
  } catch (err) {
    return;
  }

  const scene = new THREE.Scene();
  const camera = new THREE.PerspectiveCamera(42, 1, 0.1, 100);
  camera.position.set(0, 0, 14);

  const group = new THREE.Group();
  scene.add(group);

  const honey = 0xf0a81e;
  const cyan = 0x2e8fa8;
  const pale = 0xe4f2fb;

  /* 蜂巢线框：一圈圈六边形，深浅交替 */
  const R = 0.72;
  const w = Math.sqrt(3) * R;
  for (let row = -8; row <= 8; row++) {
    for (let col = -12; col <= 12; col++) {
      const x = col * w + (Math.abs(row) % 2) * (w / 2);
      const y = row * R * 1.5;
      if (Math.hypot(x, y) > 10.8) continue;
      const pts = [];
      for (let i = 0; i < 6; i++) {
        const a = Math.PI / 2 + (i * Math.PI) / 3;
        pts.push(new THREE.Vector3(Math.cos(a) * R, Math.sin(a) * R, 0));
      }
      const geo = new THREE.BufferGeometry().setFromPoints(pts);
      const strong = (row + col) % 9 === 0;
      const mat = new THREE.LineBasicMaterial({
        color: (row + col) % 7 === 0 ? cyan : honey,
        transparent: true,
        opacity: strong ? 0.36 : 0.14
      });
      const line = new THREE.LineLoop(geo, mat);
      line.position.set(x, y, Math.sin(x * 0.7 + y * 0.4) * 0.6);
      group.add(line);
    }
  }

  /* 漂浮六边形 */
  const shape = new THREE.Shape();
  for (let i = 0; i < 6; i++) {
    const a = Math.PI / 2 + (i * Math.PI) / 3;
    const px = Math.cos(a);
    const py = Math.sin(a);
    if (i === 0) shape.moveTo(px, py);
    else shape.lineTo(px, py);
  }
  shape.closePath();
  const hexGeo = new THREE.ShapeGeometry(shape);
  const floaters = [];
  const palette = [honey, cyan, pale];
  for (let i = 0; i < 16; i++) {
    const mat = new THREE.MeshBasicMaterial({
      color: palette[i % 3],
      transparent: true,
      opacity: 0.09 + (i % 4) * 0.035,
      side: THREE.DoubleSide,
      depthWrite: false
    });
    const mesh = new THREE.Mesh(hexGeo, mat);
    mesh.scale.setScalar(0.32 + Math.random() * 0.78);
    mesh.position.set((Math.random() - 0.5) * 22, (Math.random() - 0.5) * 12, -1 - Math.random() * 6);
    mesh.rotation.z = Math.random() * Math.PI;
    mesh.userData = {
      baseY: mesh.position.y,
      phase: Math.random() * Math.PI * 2,
      speed: 0.22 + Math.random() * 0.5
    };
    group.add(mesh);
    floaters.push(mesh);
  }

  /* 细碎光点 */
  const count = window.innerWidth < 760 ? 80 : 160;
  const positions = new Float32Array(count * 3);
  for (let i = 0; i < count; i++) {
    positions[i * 3] = (Math.random() - 0.5) * 30;
    positions[i * 3 + 1] = (Math.random() - 0.5) * 18;
    positions[i * 3 + 2] = -8 + Math.random() * 10;
  }
  const pointGeo = new THREE.BufferGeometry();
  pointGeo.setAttribute("position", new THREE.BufferAttribute(positions, 3));
  const pointMat = new THREE.PointsMaterial({
    color: honey,
    size: 0.055,
    transparent: true,
    opacity: 0.55,
    depthWrite: false
  });
  const points = new THREE.Points(pointGeo, pointMat);
  group.add(points);

  group.rotation.set(-0.62, 0, -0.06);

  let pointerX = 0;
  let pointerY = 0;
  let targetX = 0;
  let targetY = 0;
  window.addEventListener("pointermove", function (e) {
    pointerX = e.clientX / window.innerWidth - 0.5;
    pointerY = e.clientY / window.innerHeight - 0.5;
  }, { passive: true });

  function resize() {
    const rect = canvas.getBoundingClientRect();
    const width = Math.max(1, rect.width);
    const height = Math.max(1, rect.height);
    renderer.setPixelRatio(Math.min(window.devicePixelRatio || 1, 1.75));
    renderer.setSize(width, height, false);
    camera.aspect = width / height;
    camera.updateProjectionMatrix();
    if (reduceMotion) renderer.render(scene, camera);
  }
  resize();
  if ("ResizeObserver" in window) new ResizeObserver(resize).observe(canvas);
  else window.addEventListener("resize", resize);

  let visible = true;
  let raf = 0;

  function frame(now) {
    if (!visible || document.hidden) {
      raf = 0;
      return;
    }
    const t = now * 0.001;
    targetX += (pointerX - targetX) * 0.035;
    targetY += (pointerY - targetY) * 0.035;
    group.rotation.y = targetX * 0.24 + Math.sin(t * 0.13) * 0.05;
    group.rotation.x = -0.62 + targetY * 0.12;
    group.position.y = Math.sin(t * 0.25) * 0.15;
    floaters.forEach(function (mesh) {
      mesh.position.y = mesh.userData.baseY + Math.sin(t * mesh.userData.speed + mesh.userData.phase) * 0.5;
      mesh.rotation.z += 0.0015;
    });
    points.rotation.y = Math.sin(t * 0.05) * 0.1;
    renderer.render(scene, camera);
    raf = requestAnimationFrame(frame);
  }

  function start() {
    if (!raf && !reduceMotion) raf = requestAnimationFrame(frame);
  }

  if ("IntersectionObserver" in window) {
    new IntersectionObserver(function (entries) {
      visible = entries[0].isIntersecting;
      if (visible) start();
    }).observe(canvas);
  }
  document.addEventListener("visibilitychange", function () {
    if (!document.hidden) start();
  });

  if (reduceMotion) renderer.render(scene, camera);
  else start();
}
