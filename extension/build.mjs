// Regroupe les trois scripts dans dist/ et y copie le manifeste (version prise de package.json), la
// page d'options et son style. dist/ est ce que `web-ext lint` et `web-ext sign` lisent.
import { build } from "esbuild";
import { cp, mkdir, readFile, rm, writeFile } from "node:fs/promises";

const root = new URL(".", import.meta.url);
const dist = new URL("dist/", root);

await rm(dist, { recursive: true, force: true });
await mkdir(dist, { recursive: true });

await build({
  entryPoints: ["src/background.ts", "src/content.ts", "src/options.ts"].map((p) => new URL(p, root).pathname),
  outdir: dist.pathname,
  bundle: true,
  format: "iife",
  target: "firefox142",
  charset: "utf8",
  legalComments: "none",
  logLevel: "warning",
});

const { version } = JSON.parse(await readFile(new URL("package.json", root), "utf8"));
const manifest = JSON.parse(await readFile(new URL("static/manifest.json", root), "utf8"));
manifest.version = version;
await writeFile(new URL("manifest.json", dist), `${JSON.stringify(manifest, null, 2)}\n`);
for (const file of ["options.html", "options.css", "icon.svg"]) {
  await cp(new URL(`static/${file}`, root), new URL(file, dist));
}
