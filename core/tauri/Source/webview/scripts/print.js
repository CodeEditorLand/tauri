// Copyright 2019-2024 Tauri Programme within The Commons Conservancy
// SPDX-License-Identifier: Apache-2.0
// SPDX-License-Identifier: MIT

window.print = function () {
	return window.__TAURI_INTERNALS__.invoke("plugin:webview|print");
};
