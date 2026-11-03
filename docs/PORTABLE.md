# Capstan Portable Mode

## English

Keep the complete `runtime/` directory when extracting or moving the app. It
contains the verified Microsoft ConPTY package, native-host companions, package
manifest and MIT license. Copying only the executable omits this runtime.

The portable archive contains `Capstan.exe`, `capstan.portable`, the GPL license, third-party notices, and this guide. On first launch, Capstan creates a `data` directory beside the executable and checks that it can write and remove a temporary file there. If that check fails, startup stops with an error. The process working directory and an incidental `data` folder never select portable mode; the exact sibling marker is required.

The encrypted vault is stored at `data/vault.capstan`. Keep the complete `data` directory when moving or backing up a portable copy. For recovery, copy the directory while Capstan is closed, or use the in-app `.cbak` export. The `.cbak` password is independent of the vault master password.

The NSIS installation uses `%APPDATA%\Capstan` and does not place the vault under the program directory. Upgrading or uninstalling Capstan does not intentionally remove that directory. Back up the vault before manually deleting application data.

Windows WebView2 Runtime is required. The NSIS installer uses Tauri's bootstrapper behavior when the runtime is absent; the portable archive does not contain a WebView2 runtime. This project currently produces unsigned artifacts. Verify the downloaded archive's SHA-256 against the build output before use.

To build the x64 NSIS installer and portable archive from a Windows developer prompt:

```powershell
pnpm -C ui install --frozen-lockfile
pnpm bundle:nsis
pnpm portable:package
```

Artifacts are written under `target/release/bundle/nsis` and `target/bundle/portable`. The portable packaging script refuses to overwrite an existing archive.

## 中文

解压或移动程序时请保留完整 `runtime/` 目录，其中包含已核验的 Microsoft ConPTY、
原生宿主 companion、包清单和 MIT 许可证。只复制 exe 会遗漏捆绑运行时。

便携压缩包包含 `Capstan.exe`、`capstan.portable`、GPL 许可证、第三方声明和本说明。首次启动时，Capstan 会在程序旁创建 `data` 目录，并实际验证能否在其中写入和删除临时文件；验证失败时启动报错。只有程序同目录下内容完全匹配的标记文件才会启用便携模式，当前工作目录或碰巧存在的 `data` 文件夹不会改变模式。

加密库保存在 `data/vault.capstan`。移动或备份便携版时，请在 Capstan 关闭后保留完整 `data` 目录；也可使用应用内 `.cbak` 导出恢复。`.cbak` 口令独立于 Vault 主密码。

NSIS 安装版把数据放在 `%APPDATA%\Capstan`，不会放进程序目录。升级或卸载不会主动删除该目录；手动删除应用数据前请先备份 Vault。

Windows 需要 WebView2 Runtime。缺少运行时的安装版使用 Tauri bootstrapper 流程；便携压缩包不包含 WebView2 Runtime。本项目当前生成的构建产物没有代码签名。使用前请核对构建输出中的 SHA-256。

在 Windows 开发环境中构建 x64 NSIS 安装包和便携包：

```powershell
pnpm -C ui install --frozen-lockfile
pnpm bundle:nsis
pnpm portable:package
```

产物位于 `target/release/bundle/nsis` 和 `target/bundle/portable`。便携打包脚本不会覆盖已有压缩包。
