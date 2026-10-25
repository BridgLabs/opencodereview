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
document.querySelectorAll(".reveal, .stagger, .ba-card.after, #how-steps, .receipt, .results").forEach((element) => seen.observe(element));

// The app screenshot leans back and straightens as it scrolls into view.
const frame = document.getElementById("frame");
if (frame && motion) {
  const tilt = () => {
    const box = frame.getBoundingClientRect();
    const progress = Math.min(1, Math.max(0, (innerHeight - box.top) / (innerHeight * 0.9)));
    frame.style.transform = `perspective(1600px) rotateX(${(1 - progress) * 16}deg) scale(${0.94 + progress * 0.06})`;
  };
  addEventListener("scroll", () => requestAnimationFrame(tilt), { passive: true });
  tilt();
}

// The mascot peeking over the screenshot follows the pointer with its eyes.
const eyes = document.getElementById("peek-eyes");
const peek = document.getElementById("peek");
if (eyes && peek && motion) {
  addEventListener("pointermove", (event) => {
    const box = peek.getBoundingClientRect();
    const dx = event.clientX - (box.left + box.width / 2);
    const dy = event.clientY - (box.top + box.height / 2);
    const distance = Math.hypot(dx, dy) || 1;
    const reach = Math.min(1, distance / 400);
    eyes.style.transform = `translate(${(dx / distance) * 7 * reach}px, ${(dy / distance) * 5 * reach}px)`;
  }, { passive: true });
}

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

// No backend: the contact form opens the visitor's email app with the message filled in.
const contact = document.getElementById("contact");
if (contact) {
  contact.addEventListener("submit", (event) => {
    event.preventDefault();
    const data = new FormData(contact);
    const email = contact.elements.email;
    const valid = email.value.trim() !== "" && email.checkValidity();
    email.classList.toggle("invalid", !valid);
    if (!valid) { email.focus(); return; }
    const subject = `OpenCodeReview for teams: ${data.get("topic")}`;
    const body = [
      `Name: ${data.get("name") || "-"}`,
      `Email: ${data.get("email")}`,
      `Company: ${data.get("company") || "-"}`,
      `Developers: ${data.get("size")}`,
      "",
      data.get("message") || "",
    ].join("\n");
    location.href = `mailto:contato@bridglabs.com?subject=${encodeURIComponent(subject)}&body=${encodeURIComponent(body)}`;
    document.getElementById("contact-note").textContent = "Your email app should open now. If it doesn't, write to contato@bridglabs.com.";
  });
}
