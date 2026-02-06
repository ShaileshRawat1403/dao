const fs = require("fs");
const path = require("path");
const https = require("https");
const { execSync } = require("child_process");

const VERSION = "v0.1.1"; // This should match the package.json version
const REPO = "ShaileshRawat1403/dao";

const PLATFORM_MAP = {
  darwin: "apple-darwin",
  linux: "unknown-linux-gnu",
  win32: "pc-windows-msvc",
};

const ARCH_MAP = {
  x64: "x86_64",
  arm64: "aarch64",
};

const platform = PLATFORM_MAP[process.platform];
const arch = ARCH_MAP[process.arch];

if (!platform || !arch) {
  console.error(`Unsupported platform: ${process.platform} ${process.arch}`);
  process.exit(1);
}

if (process.platform === "linux" && process.arch === "arm64") {
  console.error("Linux ARM64 is not currently supported.");
  process.exit(1);
}

const ext = process.platform === "win32" ? "zip" : "tar.gz";
const binaryName = process.platform === "win32" ? "dao.exe" : "dao";
const artifactName = `dao-cli-${VERSION}-${arch}-${platform}.${ext}`;
const url = `https://github.com/${REPO}/releases/download/${VERSION}/${artifactName}`;

const binDir = path.join(__dirname, "bin");
if (!fs.existsSync(binDir)) {
  fs.mkdirSync(binDir);
}

const dest = path.join(binDir, artifactName);
const finalBin = path.join(binDir, binaryName);

console.log(`Downloading DAO from ${url}...`);

const file = fs.createWriteStream(dest);
https.get(url, (response) => {
  if (response.statusCode !== 200) {
    console.error(`Failed to download: ${response.statusCode}`);
    process.exit(1);
  }
  response.pipe(file);
  file.on("finish", () => {
    file.close();
    console.log("Extracting...");
    if (process.platform === "win32") {
      execSync(`tar -xf ${dest} -C ${binDir}`);
    } else {
      execSync(`tar -xzf ${dest} -C ${binDir}`);
    }
    fs.chmodSync(finalBin, 0o755);
    console.log("DAO installed successfully.");
  });
});
