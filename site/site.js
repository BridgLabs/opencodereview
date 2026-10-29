const motion = !matchMedia("(prefers-reduced-motion: reduce)").matches;
const nav = document.getElementById("nav");
addEventListener("scroll", () => nav.classList.toggle("scrolled", scrollY > 8), { passive: true });

document.querySelectorAll("[data-copy]").forEach((button) => button.addEventListener("click", async () => {
  try {
    await navigator.clipboard.writeText(button.dataset.copy);
    button.setAttribute("aria-label", "Copied");
    button.classList.add("copied");
  } catch {}
}));

// Sections fade up once; children of .stagger follow one after another.
document.querySelectorAll(".stagger").forEach((list) => [...list.children].forEach((child, index) => child.style.setProperty("--i", index)));
const seen = new IntersectionObserver((entries) => entries.forEach((entry) => {
  if (!entry.isIntersecting) return;
  entry.target.classList.add("in");
  seen.unobserve(entry.target);
}), { rootMargin: "0px 0px -12% 0px" });
document.querySelectorAll(".reveal, .stagger, .showcase").forEach((element) => seen.observe(element));

// Honest arithmetic: developers × price × 12, with the visitor's own numbers.
const devs = document.getElementById("devs");
const seat = document.getElementById("seat");
const devsOut = document.getElementById("devs-out");
const them = document.getElementById("calc-them");
if (devs && seat && them) {
  const money = new Intl.NumberFormat("en-US", { style: "currency", currency: "USD", maximumFractionDigits: 0 });
  let shown = 0;
  let frameId = 0;
  const update = () => {
    devsOut.textContent = devs.value;
    const target = Math.max(0, Number(devs.value) * Number(seat.value || 0) * 12);
    cancelAnimationFrame(frameId);
    if (!motion) { shown = target; them.textContent = money.format(target); return; }
    const from = shown;
    const start = performance.now();
    const step = (now) => {
      const t = Math.min(1, (now - start) / 450);
      shown = from + (target - from) * (1 - (1 - t) ** 3);
      them.textContent = money.format(shown);
      if (t < 1) frameId = requestAnimationFrame(step);
    };
    frameId = requestAnimationFrame(step);
  };
  devs.addEventListener("input", update);
  seat.addEventListener("input", update);
  update();
}
