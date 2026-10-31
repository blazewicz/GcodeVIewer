#!/usr/bin/env node
// Verifies the SEO metadata of the built site in ui/dist. `just ui-build` runs
// this after `vite build`, so a deployment fails when a required tag
// disappears, an image is missing, or the canonical URL drifts.

import { readFileSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const uiDir = path.dirname(path.dirname(fileURLToPath(import.meta.url)));
const distDir = path.join(uiDir, "dist");

// Must match ui/index.html, ui/public/sitemap.xml and ui/public/robots.txt.
const SITE_URL = "https://blazewicz.github.io/GcodeVIewer/";

const problems = [];

function check(condition, message) {
  if (!condition) {
    problems.push(message);
  }
}

function readDist(relative) {
  const file = path.join(distDir, relative);
  try {
    return readFileSync(file, "utf8");
  } catch {
    console.error(`Missing ${path.relative(process.cwd(), file)}; run \`npm run build\` first.`);
    process.exit(1);
  }
}

function decodeEntities(text) {
  return text
    .replaceAll("&amp;", "&")
    .replaceAll("&lt;", "<")
    .replaceAll("&gt;", ">")
    .replaceAll("&quot;", '"')
    .replaceAll("&#39;", "'");
}

function collectTags(html, tagName) {
  const tags = [];
  for (const match of html.matchAll(new RegExp(`<${tagName}\\b[^>]*>`, "gi"))) {
    const attributes = {};
    for (const attribute of match[0].matchAll(/([a-zA-Z][\w:-]*)\s*=\s*"([^"]*)"/g)) {
      attributes[attribute[1].toLowerCase()] = decodeEntities(attribute[2]);
    }
    tags.push(attributes);
  }
  return tags;
}

function pngSize(relative) {
  let buffer;
  try {
    buffer = readFileSync(path.join(distDir, relative));
  } catch {
    return null;
  }
  if (buffer.length < 24 || buffer.subarray(0, 8).toString("hex") !== "89504e470d0a1a0a") {
    return { notPng: true };
  }
  return { width: buffer.readUInt32BE(16), height: buffer.readUInt32BE(20) };
}

const html = readDist("index.html");

check(/<html[^>]*\slang="en"/.test(html), 'missing <html lang="en">');

const titleMatch = html.match(/<title>([^<]*)<\/title>/i);
const title = titleMatch ? decodeEntities(titleMatch[1]).trim() : "";
check(title.length >= 15 && title.length <= 65, `title should have 15-65 characters, found ${title.length}: ${JSON.stringify(title)}`);

const meta = new Map();
for (const tag of collectTags(html, "meta")) {
  if (tag.name) {
    meta.set(tag.name.toLowerCase(), tag.content ?? "");
  }
  if (tag.property) {
    meta.set(tag.property.toLowerCase(), tag.content ?? "");
  }
}

const description = meta.get("description") ?? "";
check(description.length >= 50 && description.length <= 160, `meta description should have 50-160 characters, found ${description.length}`);

const expectedMeta = {
  "og:type": "website",
  "og:site_name": "GcodeViewer",
  "og:url": SITE_URL,
  "og:title": title,
  "og:image": `${SITE_URL}og.png`,
  "og:image:type": "image/png",
  "og:image:width": "1200",
  "og:image:height": "630",
  "og:locale": "en_US",
  "twitter:card": "summary_large_image",
  "twitter:title": title,
};

for (const [name, expected] of Object.entries(expectedMeta)) {
  check(meta.get(name) === expected, `expected ${name}=${JSON.stringify(expected)}, found ${JSON.stringify(meta.get(name) ?? null)}`);
}

for (const name of ["og:description", "og:image:alt", "twitter:description", "twitter:image"]) {
  check((meta.get(name) ?? "").length > 0, `missing meta ${name}`);
}

for (const name of ["og:description", "twitter:description"]) {
  check(meta.get(name) === description, `${name} does not match the meta description`);
}

const canonical = collectTags(html, "link").find((tag) => tag.rel === "canonical")?.href ?? null;
check(canonical === SITE_URL, `canonical link should be ${SITE_URL}, found ${JSON.stringify(canonical)}`);

check((meta.get("google-site-verification") ?? "").length > 0, "missing google-site-verification meta");

const jsonLd = html.match(/<script type="application\/ld\+json">([\s\S]*?)<\/script>/i);
check(jsonLd !== null, "missing application/ld+json block");
if (jsonLd) {
  let data = null;
  try {
    data = JSON.parse(jsonLd[1]);
  } catch (error) {
    problems.push(`JSON-LD does not parse: ${error.message}`);
  }
  if (data) {
    check(data["@type"] === "WebApplication", `JSON-LD @type should be WebApplication, found ${JSON.stringify(data["@type"])}`);
    check(data.url === SITE_URL, "JSON-LD url does not match the canonical URL");
    check(data.description === description, "JSON-LD description does not match the meta description");
    check(data.screenshot === `${SITE_URL}og.png`, "JSON-LD screenshot does not point at og.png");
  }
}

const preload = collectTags(html, "link").find((tag) => tag.rel === "preload" && tag.as === "fetch");
check(preload !== undefined, "missing preload link for the wasm binary");
if (preload) {
  const href = preload.href ?? "";
  check(preload.type === "application/wasm", `wasm preload should have type="application/wasm", found ${JSON.stringify(preload.type ?? null)}`);
  check(href.endsWith(".wasm"), `wasm preload should point at a .wasm file, found ${JSON.stringify(href)}`);
  try {
    readFileSync(path.join(distDir, href.replace(/^\.\//, "")));
  } catch {
    problems.push(`wasm preload points at a missing file: ${href}`);
  }
}

const images = [
  ["og.png", 1200, 630],
  ["favicon-32x32.png", 32, 32],
  ["apple-touch-icon.png", 180, 180],
];

for (const [file, width, height] of images) {
  const size = pngSize(file);
  if (size === null) {
    problems.push(`missing ${file}`);
  } else if (size.notPng) {
    problems.push(`${file} is not a PNG`);
  } else {
    check(size.width === width && size.height === height, `${file} should be ${width}x${height}, found ${size.width}x${size.height}`);
  }
}

for (const file of ["favicon.svg", "robots.txt", "sitemap.xml"]) {
  try {
    readFileSync(path.join(distDir, file));
  } catch {
    problems.push(`missing ${file}`);
  }
}

const sitemap = readDist("sitemap.xml");
const loc = sitemap.match(/<loc>([^<]*)<\/loc>/)?.[1] ?? null;
check(loc === SITE_URL, `sitemap loc should be ${SITE_URL}, found ${JSON.stringify(loc)}`);

const robots = readDist("robots.txt");
check(robots.includes(`Sitemap: ${SITE_URL}sitemap.xml`), `robots.txt should list ${SITE_URL}sitemap.xml`);

if (problems.length > 0) {
  console.error(`SEO check failed with ${problems.length} problem(s):`);
  for (const problem of problems) {
    console.error(`  - ${problem}`);
  }
  process.exit(1);
}

console.log(`SEO check passed: ${JSON.stringify(title)}`);
