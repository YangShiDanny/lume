# Lume Language Support

为 `.lume` 文件提供 VS Code 语言支持，包括语法高亮、括号/注释编辑行为、Lume 扩展图标，以及可选的 Lume 文件图标主题。

Declarative VS Code language support for `.lume` files, including syntax highlighting, editor bracket/comment behavior, the Lume extension icon, and an optional file icon theme.

## 在 VS Code 中运行

1. 在 VS Code 中打开本 `vscode-extension` 文件夹。
2. 按 `F5` 打开扩展开发宿主（Extension Development Host）。
3. 打开任意 `.lume` 文件，即可使用 Lume 语法。
4. 想显示 Lume 文件图标，运行 **Preferences: File Icon Theme**，选择 **Lume File Icons**。

文件图标主题是可选的（opt-in），不选就不会替换你当前的图标主题。

## 打包

在本目录下运行 `npx @vscode/vsce package`，生成 `.vsix` 安装包。

发布前，请把 `package.json` 里的占位 publisher 替换成你自己的 Marketplace publisher ID。

## Run in VS Code

1. Open this `vscode-extension` folder in VS Code.
2. Press `F5` to open an Extension Development Host.
3. Open a `.lume` file to use the Lume grammar.
4. To show the Lume file icon, run **Preferences: File Icon Theme** and select **Lume File Icons**.

The file icon theme is opt-in so it does not replace a user's current icon theme unless selected.

## Package

Run `npx @vscode/vsce package` from this folder to create a `.vsix` package. Before publishing, replace the placeholder publisher in `package.json` with your Marketplace publisher ID.