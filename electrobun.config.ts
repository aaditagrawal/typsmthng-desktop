import type { ElectrobunConfig } from "electrobun";

export default {
  app: {
    name: "typsmthng",
    identifier: "dev.typsmthng.desktop",
    version: "0.1.0",
  },
  release: {
    baseUrl:
      "https://github.com/aaditagrawal/typsmthng-desktop/releases/latest/download",
  },
  build: {
    copy: {
      "dist/index.html": "views/mainview/index.html",
      "dist/assets": "views/mainview/assets",
      "src/bun/libMacWindowEffects.dylib": "bun/libMacWindowEffects.dylib",
      "assets/icon.png": "Resources/appIcon.png",
    },
    watchIgnore: ["dist/**"],
    mac: {
      bundleCEF: false,
      icons: "icon.iconset",
    },
    linux: {
      bundleCEF: false,
    },
    win: {
      bundleCEF: false,
    },
  },
} satisfies ElectrobunConfig;
