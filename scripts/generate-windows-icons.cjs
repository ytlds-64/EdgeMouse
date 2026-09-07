// Rebuild with Node.js + sharp. Every size is rasterized from the SVG master;
// ICO directory order matters because Tauri embeds the first entry at runtime.
const fs = require("node:fs");
const path = require("node:path");
const sharp = require("sharp");

async function main() {
  const root = path.resolve(__dirname, "..");
  const source = fs.readFileSync(path.join(root, "assets/branding/edgemouse-app-icon.svg"));
  const destination = path.join(root, "crates/edgemouse-desktop/icons");
  const sizes = [256, 128, 96, 64, 48, 40, 32, 24, 20, 16];
  const images = [];
  for (const size of sizes) {
    const png = await sharp(source).resize(size, size).ensureAlpha().png().toBuffer();
    const { data, info } = await sharp(png).raw().toBuffer({ resolveWithObject: true });
    for (const offset of [0, (size - 1) * 4, size * (size - 1) * 4, (size * size - 1) * 4]) {
      if (info.channels !== 4 || data[offset + 3] !== 0) throw new Error(`Opaque icon corner: ${size}`);
    }
    images.push(png);
    if (size === 64) fs.writeFileSync(path.join(destination, "tray.png"), png);
  }
  const header = Buffer.alloc(6 + 16 * sizes.length);
  header.writeUInt16LE(1, 2);
  header.writeUInt16LE(sizes.length, 4);
  let offset = header.length;
  sizes.forEach((size, index) => {
    const entry = 6 + index * 16;
    header[entry] = header[entry + 1] = size === 256 ? 0 : size;
    header.writeUInt16LE(1, entry + 4);
    header.writeUInt16LE(32, entry + 6);
    header.writeUInt32LE(images[index].length, entry + 8);
    header.writeUInt32LE(offset, entry + 12);
    offset += images[index].length;
  });
  fs.writeFileSync(path.join(destination, "icon.ico"), Buffer.concat([header, ...images]));
  console.log(`Generated transparent ICO frames: ${sizes.join(", ")}; tray: 64 px`);
}

main().catch(error => { console.error(error); process.exitCode = 1; });
