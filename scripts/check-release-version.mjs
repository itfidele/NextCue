import { readFileSync } from "node:fs";
const read = (path) => readFileSync(new URL(`../${path}`, import.meta.url), "utf8");
const version = JSON.parse(read("package.json")).version;
const tauriVersion = JSON.parse(read("src-tauri/tauri.conf.json")).version;
const cargoVersion = read("src-tauri/Cargo.toml").split(/^\[/m)[1]?.match(/^version\s*=\s*"([^"]+)"/m)?.[1];
if (version !== tauriVersion || version !== cargoVersion) {
  throw new Error(`Version mismatch: npm=${version}, Tauri=${tauriVersion}, Cargo=${cargoVersion}`);
}
if (process.env.GITHUB_REF_TYPE === "tag" && process.env.GITHUB_REF_NAME !== `v${version}`) {
  throw new Error(`Release tag must be v${version}; received ${process.env.GITHUB_REF_NAME}`);
}
console.log(`NextCue ${version}: package versions and release tag are consistent.`);
