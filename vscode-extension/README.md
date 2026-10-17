# Lume Language Support

Declarative VS Code language support for `.lume` files, including syntax highlighting, editor bracket/comment behavior, the Lume extension icon, and an optional file icon theme.

## Run in VS Code

1. Open this `vscode-extension` folder in VS Code.
2. Press `F5` to open an Extension Development Host.
3. Open a `.lume` file to use the Lume grammar.
4. To show the Lume file icon, run **Preferences: File Icon Theme** and select **Lume File Icons**.

The file icon theme is opt-in so it does not replace a user's current icon theme unless selected.

## Package

Run `npx @vscode/vsce package` from this folder to create a `.vsix` package. Before publishing, replace the placeholder publisher in `package.json` with your Marketplace publisher ID.