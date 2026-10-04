// Stats card window (pet right-click), a port of the macOS PetStatsView
// popover. Content comes from statscard.ts; this file wires data, the sprite
// thumbnail, badge hover hints, the footer, sizing and focus-loss hiding.

import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow, LogicalSize } from "@tauri-apps/api/window";
import { check } from "@tauri-apps/plugin-updater";
import { relaunch, exit } from "@tauri-apps/plugin-process";
import * as care from "./care";
import * as limits from "./limits";
import * as usage from "./usage";
import { savedSlug, getLibrary, petDisplayName } from "./catalog";
import { t, getLang, setLang, type Lang } from "./i18n";
import { cardHtml, hintText } from "./statscard";

const body = document.getElementById("sc-body")!;
// First open carries the pet in the URL (the show event can fire before this
// script loads); later opens reuse the window and send it in "stats-shown".
let petSlug: string | null = new URLSearchParams(location.search).get("pet");

function currentSlug(): string | null { return petSlug || savedSlug(); }

function petName(slug: string | null): string {
  if (!slug) return t("Your pet");
  const custom = petDisplayName(slug);
  if (custom !== slug) return custom;
  return getLibrary().find((p) => p.slug === slug)?.name || slug;
}

function petSheetUrl(slug: string | null): string | null {
  const lib = getLibrary().find((p) => p.slug === slug);
  return lib?.url || (slug === savedSlug() ? localStorage.getItem("ap_pet_url") : null);
}

/// First idle frame of the 8x9 sheet (same as Settings' drawThumb). Loaded
/// without crossOrigin: we only draw it, never read pixels back, and a
/// CORS-mode load fails on the non-CORS copy the pet window already cached
/// (see pet.ts load() retry).
function drawThumb(url: string | null) {
  const cv = document.getElementById("sc-thumb") as HTMLCanvasElement | null;
  const ctx = cv?.getContext("2d");
  const log = (msg: string) => { invoke("log_debug", { msg: `stats thumb: ${msg}` }).catch(() => {}); };
  if (!cv || !ctx || !url) { log(`skipped (canvas=${!!cv} ctx=${!!ctx} url=${url ? url.slice(0, 80) : "none"})`); return; }
  ctx.imageSmoothingEnabled = false;
  const img = new Image();
  img.onerror = () => log(`load failed ${url.slice(0, 80)}`);
  img.onload = () => {
    const fw = img.naturalWidth / 8, fh = img.naturalHeight / 9;
    if (!fw || !fh) { log(`empty image ${img.naturalWidth}x${img.naturalHeight}`); return; }
    const sc = Math.min(cv.width / fw, cv.height / fh);
    ctx.clearRect(0, 0, cv.width, cv.height);
    ctx.drawImage(img, 0, 0, fw, fh, (cv.width - fw * sc) / 2, (cv.height - fh * sc) / 2, fw * sc, fh * sc);
  };
  img.src = url;
}

let lastHtml = "";
function paint() {
  const slug = currentSlug();
  const state = slug ? care.stateFor(slug) : care.emptyState();
  const html = cardHtml({
    name: petName(slug), state, providers: limits.cached(),
    costToday: usage.todayCostUSD(), costMonth: usage.monthlyCostUSD(), lang: getLang(),
  });
  if (html === lastHtml) return; // keep hover state + thumbnail when nothing changed
  lastHtml = html;
  body.innerHTML = html;
  drawThumb(petSheetUrl(slug));
  const hint = document.getElementById("sc-hint")!;
  const idle = hint.textContent || "";
  const unlocked = new Set(state.unlockedAchievements || []);
  body.querySelectorAll<HTMLElement>(".sc-badge").forEach((b) => {
    b.onmouseenter = () => { hint.textContent = hintText(b.dataset.ach!, unlocked.has(b.dataset.ach!)); hint.classList.add("on"); };
    b.onmouseleave = () => { hint.textContent = idle; hint.classList.remove("on"); };
  });
  fit();
}

let lastH = 0;
function fit() {
  const card = document.querySelector(".sc-card") as HTMLElement;
  const h = Math.min(900, Math.max(240, card.scrollHeight + 20));
  if (Math.abs(h - lastH) < 2) return;
  lastH = h;
  getCurrentWindow().setSize(new LogicalSize(316, h)).catch(() => {});
}

function applyStatic() {
  const set = (id: string, key: string) => { const el = document.getElementById(id); if (el) el.textContent = t(key); };
  set("t-sc-settings", "Settings");
  set("t-sc-updates", "Updates");
  set("t-sc-quit", "Quit");
}

(document.getElementById("sc-settings") as HTMLButtonElement).onclick = () => {
  invoke("open_settings").catch(() => {});
  void getCurrentWindow().hide();
};
(document.getElementById("sc-quit") as HTMLButtonElement).onclick = () => { exit(0); };
const upd = document.getElementById("sc-updates") as HTMLButtonElement;
upd.onclick = async () => {
  const label = document.getElementById("t-sc-updates")!;
  const reset = () => setTimeout(() => { label.textContent = t("Updates"); }, 2500);
  label.textContent = t("Checking…");
  try {
    const update = await check();
    if (update) { label.textContent = t("Installing…"); await update.downloadAndInstall(); await relaunch(); }
    else { label.textContent = t("Up to date"); reset(); }
  } catch { label.textContent = t("Up to date"); reset(); }
};

// Transient like the mac popover: hide on focus loss, Escape, or a pet click.
getCurrentWindow().onFocusChanged(({ payload: focused }) => { if (!focused) void getCurrentWindow().hide(); });
listen("popover-close", () => void getCurrentWindow().hide());
window.addEventListener("keydown", (e) => { if (e.key === "Escape") void getCurrentWindow().hide(); });
// No WebView2 Back/Refresh/Print menu inside the card.
window.addEventListener("contextmenu", (e) => e.preventDefault());

// Rust emits this right before showing the card; the pet window that was
// right-clicked says which pet (split-pet windows each have their own).
listen<string | null>("stats-shown", (e) => { petSlug = e.payload || null; lastHtml = ""; paint(); });
listen("care-updated", () => paint());
listen<Lang>("lang-changed", (e) => { setLang(e.payload); applyStatic(); lastHtml = ""; paint(); });
setInterval(paint, 30_000); // hunger + "last fed" clocks

applyStatic();
paint();
