#!/usr/bin/env node
// Regenerates the committed social preview (ui/public/og.png) and the PNG icon
// set from a headless Chromium render of the built UI (ui/dist).
//
// `just og-image` installs Playwright and Chromium and runs this script; the
// browser download only happens on the first run.
//
//   node scripts/og-image.mjs            screenshot the demo toolpath
//   node scripts/og-image.mjs --branded  draw the fallback card instead

import { createServer } from "node:http";
import { existsSync } from "node:fs";
import { readFile, writeFile } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";

const uiDir = path.dirname(path.dirname(fileURLToPath(import.meta.url)));
const distDir = path.join(uiDir, "dist");
const publicDir = path.join(uiDir, "public");

const MIME_TYPES = new Map(
  Object.entries({
    ".css": "text/css",
    ".gcode": "text/plain; charset=utf-8",
    ".html": "text/html; charset=utf-8",
    ".ico": "image/x-icon",
    ".js": "text/javascript",
    ".json": "application/json",
    ".png": "image/png",
    ".svg": "image/svg+xml",
    ".txt": "text/plain; charset=utf-8",
    ".wasm": "application/wasm",
    ".xml": "application/xml",
  }),
);

/** Serves ui/dist on a random loopback port: the page fetches its own files. */
async function startServer() {
  const server = createServer(async (request, response) => {
    const pathname = decodeURIComponent(new URL(request.url ?? "/", "http://localhost").pathname);
    const file = path.resolve(distDir, pathname.replace(/^\/+/, "") || "index.html");

    if (!file.startsWith(distDir + path.sep)) {
      response.writeHead(403).end();
      return;
    }

    try {
      const body = await readFile(file);
      response.writeHead(200, {
        "content-type": MIME_TYPES.get(path.extname(file)) ?? "application/octet-stream",
      });
      response.end(body);
    } catch {
      response.writeHead(404).end("not found");
    }
  });

  await new Promise((resolve, reject) => {
    server.once("error", reject);
    server.listen(0, "127.0.0.1", resolve);
  });

  return { server, port: server.address().port };
}

async function loadPlaywright() {
  try {
    return await import("playwright");
  } catch {
    console.error("Playwright is not installed. Run `just og-image`, or:");
    console.error("  npm --prefix ui install --no-save playwright");
    console.error("  npm --prefix ui exec -- playwright install chromium");
    process.exit(1);
  }
}

async function faviconDataUrl() {
  const svg = await readFile(path.join(publicDir, "favicon.svg"), "utf8");
  return `data:image/svg+xml;base64,${Buffer.from(svg).toString("base64")}`;
}

function brandedCardHtml(markDataUrl) {
  return `<!doctype html>
<html lang="en">
  <head>
    <meta charset="utf-8" />
    <style>
      html, body { margin: 0; }
      body {
        box-sizing: border-box;
        width: 1200px;
        height: 630px;
        display: flex;
        align-items: center;
        gap: 64px;
        padding: 72px;
        background: #f7f8fa;
        color: #262c3a;
        font-family: ui-sans-serif, system-ui, sans-serif;
      }
      .mark { flex: none; }
      h1 { margin: 0 0 16px; font-size: 76px; letter-spacing: -1px; }
      p { margin: 0; }
      .tagline { font-size: 30px; color: #5c6577; }
      .features { margin-top: 28px; font-size: 23px; color: #78808e; }
    </style>
  </head>
  <body>
    <img class="mark" src="${markDataUrl}" width="288" height="288" alt="" />
    <div>
      <h1>GcodeViewer</h1>
      <p class="tagline">Browser-based G-code toolpath preview</p>
      <p class="features">Color-coded moves &middot; Feedrate coloring &middot; Z clipping</p>
    </div>
  </body>
</html>`;
}

/** Loads the built site, loads the demo toolpath and leaves it rendered. */
async function loadDemoToolpath(page, port) {
  page.on("pageerror", (error) => console.warn(`browser error: ${error.message}`));
  page.on("console", (message) => {
    if (message.type() === "error") {
      console.warn(`browser console: ${message.text()}`);
    }
  });

  await page.goto(`http://127.0.0.1:${port}/`, { waitUntil: "load" });

  // The controls exist only once the renderer was created; a failed startup
  // shows up in #stats instead.
  await page.waitForFunction(
    () => {
      const stats = document.querySelector("#stats")?.textContent ?? "";
      return stats.startsWith("Renderer failed") || (document.querySelector("#sliders")?.childElementCount ?? 0) > 0;
    },
    undefined,
    { timeout: 60_000 },
  );

  const startupStats = ((await page.textContent("#stats")) ?? "").trim();
  if (!startupStats.startsWith("Renderer failed")) {
    await page.click("#demo");
    await page.waitForFunction(
      () => {
        const stats = document.querySelector("#stats")?.textContent ?? "";
        return stats.includes("moves") || stats.startsWith("Loading failed") || stats.startsWith("Render failed");
      },
      undefined,
      { timeout: 120_000 },
    );
  }

  const loadedStats = ((await page.textContent("#stats")) ?? "").trim();
  if (!loadedStats.includes("moves")) {
    throw new Error(loadedStats || "the demo toolpath did not load");
  }

  // Give the frame loop two frames to paint the freshly loaded toolpath.
  await page.evaluate(() => new Promise((resolve) => requestAnimationFrame(() => requestAnimationFrame(resolve))));

  // The loader fits the view to the move endpoints, so arcs that bulge past
  // their endpoints start slightly cropped; one wheel scroll below the canvas
  // centre zooms out enough to frame the whole demo toolpath.
  await page.mouse.move(480, 575);
  await page.mouse.wheel(0, 284);
  await page.evaluate(() => new Promise((resolve) => requestAnimationFrame(() => requestAnimationFrame(resolve))));
}

async function iconScreenshot(browser, { size, background }) {
  const page = await browser.newPage({ viewport: { width: size, height: size } });
  const style = `margin:0;overflow:hidden;background:${background ?? "transparent"}`;
  await page.setContent(
    `<!doctype html><html><body style="${style}"><img src="${await faviconDataUrl()}" width="${size}" height="${size}" alt=""></body></html>`,
  );
  const buffer = await page.screenshot({ omitBackground: background === undefined });
  await page.close();
  return buffer;
}

if (!existsSync(path.join(distDir, "index.html"))) {
  console.error("ui/dist is missing; run `just ui-build` first.");
  process.exit(1);
}

const brandedOnly = process.argv.includes("--branded");
const { chromium } = await loadPlaywright();
const { server, port } = await startServer();
const browser = await chromium.launch();

try {
  const page = await browser.newPage({ viewport: { width: 1200, height: 630 }, deviceScaleFactor: 1 });

  let source = "the demo toolpath";
  if (brandedOnly) {
    source = "the branded card (--branded)";
    await page.setContent(brandedCardHtml(await faviconDataUrl()));
  } else {
    try {
      await loadDemoToolpath(page, port);
    } catch (error) {
      console.warn(`Live render unavailable (${error.message}); using the branded card instead.`);
      source = "the branded card";
      await page.setContent(brandedCardHtml(await faviconDataUrl()));
    }
  }

  await writeFile(path.join(publicDir, "og.png"), await page.screenshot());
  await page.close();

  await writeFile(path.join(publicDir, "favicon-32x32.png"), await iconScreenshot(browser, { size: 32 }));
  await writeFile(path.join(publicDir, "apple-touch-icon.png"), await iconScreenshot(browser, { size: 180, background: "#262c3a" }));

  console.log(`Wrote og.png (1200x630) from ${source}.`);
  console.log("Wrote favicon-32x32.png (32x32) and apple-touch-icon.png (180x180) from favicon.svg.");
} finally {
  await browser.close();
  server.close();
}
