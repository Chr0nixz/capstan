# M1 维护者重点手测（2026-10-16）

**最新状态（2026-10-17）：M1 已 accepted，无必须重做的手测。** M0 剩余实机范围也经维护者确认；工程与性能交付已补齐，见 [M1-B](M1-B.md)。下方旧包、测试步骤与待交付描述按 2026-10-16 历史保留。新包为 `capstan-0.1.0-win-x64-m1-final-20261017.zip`；既有 C998 目录已备份更新程序、完整 runtime 和说明，数据保持不变。

**最新验收：** 自动化收尾后，维护者进一步反馈“实机测试全部通过”，并说明原始记录“未保存”。剩余 M1 实机功能清单已登记通过，[来源与范围](baselines/M1/raw/2026-10-16-maintainer-full-manual/README.md) 已归档；无需重复本页任何已报告测试。未保存的性能原始数据与 M0 前置工程材料是交付缺口，不能用本条反馈生成数值、包来源或版本。下方步骤仅保留复现参考。

本轮先确认影响后续开发的三个桌面流程：P1 加密库与真实终端闭环、P2 Windows OpenSSH Agent 转发、P3 主机换钥阻断与恢复。后端真实 OpenSSH、Pageant、密码/四种 Ed25519 文件认证和换钥测试已有成功证据，不要求维护者重跑 Rust/UI 测试。

这份清单是优先范围，不等于整个 M1 契约。其余维护者验收列在文末；2026-10-16 代码收尾已替换 RSA 后端并移除漏洞依赖/审计例外，不再要求维护者处理旧 RSA 阻断。自动化和交付结果见 [M1-B](M1-B.md)。

**本轮结果：** 2026-10-16 维护者反馈“测试全部通过”，P1/P2/P3 必测范围均已记录通过，先前 P1 第 4 步失败项已关闭。[验收记录](baselines/M1/raw/2026-10-16-maintainer-priority/README.md) 保留反馈来源与范围；以下步骤留作复现参考，无需重复本轮重点。可选追加项和第 7 节的暂缓项目保留各自状态。

**0. 清理旧运行遗留，选择正确的包**

2026-10-16 收尾检查确认旧遗留目录 `D:\Projects\capstan\target\m1-sshd-56dbde153625` 已不存在，不需要继续执行此前那条删除指示。现有 C998 手测环境保留；不要上传临时密钥、口令或 Vault。

当前收尾候选为 [capstan-0.1.0-win-x64-m1-closeout-20261016.zip](../../target/bundle/portable/capstan-0.1.0-win-x64-m1-closeout-20261016.zip)，SHA-256：

```text
785D1EA6F18F27880C90652F3942D98085E9926C059CC6AC095F547EBD87E2E6
```

本包包含终端高度、启动、P1 信任确认超时、SSH 错误误报本地 Shell、RSA 后端替换和发布 CSP 修复。若已按旧指南建立测试环境，更新时只替换 `Capstan.exe`，保留原 `capstan.portable` 和完整 `data`。本包未签名，仅用于受控测试。

2026-10-16 收尾时已备份、更新并重开 `target/m1-manual-app-c9982bd2779b/Capstan.exe`，此目录无需自行替换。Vault、备份、锁文件和便携标记哈希未变。P1/P2/P3 已验收，不必重复下面的准备或测试；以下步骤用于以后复现。

**1. 一次性准备：新便携目录和本机测试 sshd**

使用普通权限 PowerShell，复制执行下面的整段命令，并保留这个窗口，后面的换钥与清理命令复用其中变量。已探测到本机 WSL 发行版名为 `Ubuntu`。该流程生成随机账户和测试密钥，只监听 `127.0.0.1:22292`；没有系统 sshd 时解压发行版软件包执行，不安装系统服务，不改 `/etc/ssh/sshd_config` 或用户 `known_hosts`。

```powershell
Set-Location -LiteralPath 'D:\Projects\capstan'
$ErrorActionPreference = 'Stop'
$m1Zip = 'D:\Projects\capstan\target\bundle\portable\capstan-0.1.0-win-x64-m1-closeout-20261016.zip'
if ((Get-FileHash -LiteralPath $m1Zip).Hash -ne '785D1EA6F18F27880C90652F3942D98085E9926C059CC6AC095F547EBD87E2E6') {
    throw 'The manual-test archive does not match the verified build.'
}
$m1Token = [Guid]::NewGuid().ToString('N').Substring(0, 12)
$m1Account = "cm1$m1Token"
$m1Fixture = "/var/tmp/capstan-m1-ssh-$m1Token"
$m1Secrets = "D:\Projects\capstan\target\m1-sshd-$m1Token\secrets"
$m1SecretsWsl = "/mnt/d/Projects/capstan/target/m1-sshd-$m1Token/secrets"
$m1Script = '/mnt/d/Projects/capstan/scripts/m1-sshd-fixture.sh'
$m1App = "D:\Projects\capstan\target\m1-manual-app-$m1Token"
Expand-Archive -LiteralPath $m1Zip -DestinationPath $m1App
New-Item -ItemType Directory -Path $m1Secrets | Out-Null
$m1Acl = [System.Security.AccessControl.DirectorySecurity]::new()
$m1Acl.SetAccessRuleProtection($true, $false)
$m1Sid = [System.Security.Principal.WindowsIdentity]::GetCurrent().User
$m1Rule = [System.Security.AccessControl.FileSystemAccessRule]::new(
    $m1Sid, 'FullControl', 'ContainerInherit,ObjectInherit', 'None', 'Allow'
)
$m1Acl.AddAccessRule($m1Rule)
Set-Acl -LiteralPath $m1Secrets -AclObject $m1Acl
wsl.exe -d Ubuntu -u root -- sh $m1Script start $m1Fixture $m1SecretsWsl 22292 $m1Account
if ($LASTEXITCODE -ne 0) { throw 'Fixture startup failed. Run the cleanup step; do not continue connecting.' }
[pscustomobject]@{
    Address = '127.0.0.1'
    Port = 22292
    Username = $m1Account
    SecretDirectory = $m1Secrets
    ApplicationDirectory = $m1App
} | Format-List
wsl.exe -d Ubuntu -u root -- ssh-keygen -lf "$m1Fixture/host-key.pub" -E sha256
```

预期有 `STARTED`，随后打印目标、随机用户名、两个目录和 `SHA256:...` 主机指纹。记录这条指纹，用于首次连接比对。如果端口已占用、下载失败或没有 `STARTED`，不要杀其他服务或连接该端口；执行第 5 步停止本次夹具，然后只回报失败步骤和错误。

在资源管理器中打开输出的 `ApplicationDirectory`，双击 `Capstan.exe`。窗口应显示侧栏与本地终端。若看不到侧栏，点顶部“已保存的主机”。本次数据只在这个新目录的 `data` 下，不使用已有 Vault。

`SecretDirectory` 中的 `password.txt` 是远程测试账户密码；`passphrase.txt` 是加密测试私钥口令；`openssh`、`openssh-encrypted` 等是现场生成的测试密钥。只有需要录入时才在本地打开相应口令文本，把值输入 Capstan 的原生窗口；不要把秘密发到聊天或证据中。

不要用 `Test-M1Sshd.ps1` 代替上述准备：自动测试脚本结束就会删除服务器和账户，无法留给 GUI 联测。

**2. P1：创建、密码 SSH、重启、锁定（约 10 分钟）**

1. 未创建 Vault 时，在本地 PowerShell 终端键入 `Write-Output 'Capstan local ready'`，确认输出。点“创建加密库”，在原生 Windows 对话框中设置一个自己保存的测试主密码，至少 12 个字符。成功后显示“加密库已解锁”。
2. 展开“添加主机”，填“凭据名称”为 `M1-password`，点“添加密码凭据”。在弹出的原生窗口中输入 `password.txt` 中的随机密码。侧栏应只有凭据名称/类型，不显示密码。
3. 保存一台 `M1-password-host`：地址 `127.0.0.1`、端口 `22292`、用户名为准备阶段打印的 `cm1...`、凭据选 `M1-password`。其他字段先留默认。Agent 转发选项应不可用。
4. 点该主机“连接”。首次提示应显示目标 `127.0.0.1:22292`、算法 `ssh-ed25519` 和准备阶段的指纹。先点“拒绝”：不能出现可用 SSH shell。再次连接，核对同一指纹后点“信任并保存”：应进入远程 shell。
5. 在 SSH 终端中逐条输入下面的命令，检查中文/emoji、窗口改变后的行列数及 Ctrl+C。`sleep 30` 执行后按 Ctrl+C，应立即回到提示符。需要手动改窗口大小，再第二次运行 `stty size`。

   ```sh
   printf 'Capstan 中文 🧪\n'
   stty size
   sleep 30
   ```

6. 完全退出 Capstan，再启动同目录的 `Capstan.exe`。先用错误主密码解锁：应有可理解的错误，保持锁定。再用正确主密码解锁：已保存的主机和凭据应仍在；连接同一主机不应再次要求首次信任。
7. 保持一个 SSH 和一个本地终端。先在“加密库安全设置”确认“锁定加密库时断开 SSH 会话”未选中，点“锁定加密库”：已有 SSH 和本地终端应仍可输入，保存的主机/凭据界面应清空或不可访问，不能新建 SSH 连接。解锁后数据恢复。
8. 勾选“锁定加密库时断开 SSH 会话”，点“保存安全设置”，再锁定：所有 SSH 应结束，本地终端仍可输入。解锁，取消该选项并保存，为下一项恢复默认策略。

P1 通过标准：窗口正常绘制；秘密通过原生窗口录入；拒绝信任时无连接；正确认证后终端可用；错误主密码不丢数据；重启保存主机/凭据/信任；两种锁定策略符合上述表现。

如果有余力，可在同一个测试 Vault 补验“导入私钥”：选择 `openssh-encrypted`，首次故意输入错误口令，应失败且可重试；再使用 `passphrase.txt` 中的正确口令，创建独立测试主机并连接。四种文件的后端兼容已经实测，这一步补的是原生选文件/口令失败后的 GUI 恢复。

**3. P2：Windows OpenSSH Agent，关闭→开启→再次关闭（约 10 分钟）**

只有这项需要临时调整 Windows `ssh-agent` 服务。刚才探测的原状态为 `Stopped / Disabled`。先在另一个“以管理员身份运行”的 PowerShell 中查看并记录当前值，再启用；不要用管理员窗口运行 Capstan 或加载密钥。

```powershell
Get-Service -Name ssh-agent | Select-Object Status, StartType
Set-Service -Name ssh-agent -StartupType Manual
Start-Service -Name ssh-agent
```

回到第 1 步保留的普通 PowerShell，检查身份，再加载本次密钥：

```powershell
$m1SshAdd = Join-Path $env:WINDIR 'System32\OpenSSH\ssh-add.exe'
& $m1SshAdd -l
```

预期为 `The agent has no identities.`，此时非零退出正常。如果已经列出其他身份，先不要进行转发测试，也不要运行 `ssh-add -D`。P1/P3 仍可继续，把 P2 记为需要隔离 Windows 用户；转发可访问 Agent 中其他身份，不能用生产身份进行这项测试。

无其他身份时继续：

```powershell
& $m1SshAdd "$m1Secrets\openssh"
& $m1SshAdd -l
```

预期仅有一条 ED25519 身份，记录其 SHA256 指纹。Capstan 展开“添加主机”，凭据名称填 `M1-agent`，Provider 选择“Windows OpenSSH Agent”，点“读取 Agent 身份”，选择上述指纹，点“添加 Agent 凭据”。保存 `M1-agent-host`，地址/端口/用户名与 P1 相同，凭据选 `M1-agent`。

每次修改转发设置后，都保存主机、完全退出 Capstan 再启动并解锁，确保检查的是新连接。已有 SSH 连接不会因保存设置立即撤销授权。

| 操作 | 在远端 shell 中检查 | 通过标准 |
| --- | --- | --- |
| 新建 Agent 主机，保留默认关闭 | `ssh-add -l`；`ssh-add -T ~/forwarding.pub` | 能用 Agent 登录，但两条远端命令均失败，远端没有可用 Agent socket。 |
| 编辑主机，选中“允许此主机使用 SSH Agent”，保存并重启应用 | 同样两条命令；每条执行后马上输入 `printf 'exit=%s\n' "$?"` | 枚举仅有本次指纹；`ssh-add -T` 后退出码为 `0`。它成功时可以没有其他输出。 |
| 开启状态下，不关第一个标签，再点同一主机“连接”打开第二个标签 | 两个标签分别执行 `ssh-add -T ~/forwarding.pub` 并查看退出码 | 两个 shell 都能打开、签名成功；关闭第一个后，第二个仍可输入。这覆盖本轮发现的真实兼容缺陷。 |
| 取消转发，保存并重启应用 | 再执行远端枚举和签名 | 两条均再次失败；能用 Agent 登录。重启后开关仍为关闭。 |

再验证一次身份丢失与恢复：完全退出 Capstan，在普通 PowerShell 执行 `& $m1SshAdd -d "$m1Secrets\openssh"`，重新打开应用并连接 Agent 主机。应提示 Agent 身份不可用或无可用身份，不能静默用密码继续；P1 的密码主机应仍能连接。用 `& $m1SshAdd "$m1Secrets\openssh"` 加回测试身份，再新建连接应恢复。

服务重启可追加验证：退出 Capstan，在管理员窗口执行 `Restart-Service -Name ssh-agent`；普通窗口用 `ssh-add -l` 检查测试身份，若没有则只重新加载本次密钥。再次通过 GUI 读取身份并新建连接，应成功。这项与“无 Agent 服务时可理解的错误”可分别记结果；没有测到的拒签/取消路径不要一并勾选。

**4. P3：真实换钥后的 GUI 阻断与恢复（约 5 分钟）**

1. 确认 P1 的首次信任和重启连接成功，然后完全退出 Capstan，关闭所有 SSH 连接。不要只关闭一个标签，否则连接池可能继续使用旧传输。
2. 在第 1 步的普通 PowerShell 执行下面两条命令。只更换本次专用 sshd 的主机密钥，不修改其他服务。

   ```powershell
   wsl.exe -d Ubuntu -u root -- sh $m1Script rotate $m1Fixture $m1SecretsWsl 22292 $m1Account
   if ($LASTEXITCODE -ne 0) { throw 'Fixture key rotation failed. Stop here and run cleanup.' }
   wsl.exe -d Ubuntu -u root -- ssh-keygen -lf "$m1Fixture/host-key.pub" -E sha256
   ```

3. 预期 `ROTATED`，新指纹应与准备阶段不同。再次打开 Capstan、解锁，连接 `M1-password-host`。必须出现“服务器身份已变化”，同时显示正确的旧/新指纹；只能“关闭提示并保持阻断”，不能直接信任或忽略继续，不能得到可输入的 SSH shell。
4. 关闭提示，重新连接，仍须阻断。确认新指纹确实与刚才 WSL 管理窗口输出相同后，展开侧栏“SSH 主机密钥信任”，找到 `127.0.0.1:22292` 的旧记录，点“移除信任”并确认。再次连接应重新出现首次信任提示；核对新指纹后“信任并保存”才可恢复。
5. 再退出和打开一次，正确解锁后连接应成功，不重复首次信任提示。

P3 通过标准：换钥引起明确阻断；新旧指纹准确；重复连接仍阻断；只有独立核验、移除旧信任、重新确认三步后才能恢复；新信任重启后保留。认证前零请求已有自动计数回归，本项补真实桌面提示与操作验收。

**5. 测试后清理**

先完全退出测试 Capstan，再在普通 PowerShell 停止夹具：

```powershell
wsl.exe -d Ubuntu -u root -- sh $m1Script stop $m1Fixture $m1SecretsWsl 22292 $m1Account
if ($LASTEXITCODE -ne 0) { throw 'Fixture cleanup failed. Report this step before deleting the local test files.' }
```

预期 `CLEANED`。若 P2 加载过密钥，服务仍在运行时只移除本次身份：

```powershell
& $m1SshAdd -d "$m1Secrets\openssh"
```

按测试前记录恢复服务。如果测试前确实是 `Stopped / Disabled`，在管理员 PowerShell 执行：

```powershell
Stop-Service -Name ssh-agent
Set-Service -Name ssh-agent -StartupType Disabled
```

如果原值不同，恢复原值，不执行上述固定恢复。最后在资源管理器中删除本次 `target\m1-sshd-<本次 token>` 和 `target\m1-manual-app-<本次 token>` 两个目录；前者含一次性密钥，后者含测试 Vault。口令用过剪贴板时清空剪贴板及其测试条目。

**6. 只需要回报这些结果**

```text
包：m1-closeout-20261016，SHA256 匹配/不匹配
日期、Windows/WebView2/OpenSSH 版本：
P1：创建/原生密码/信任拒绝/终端/错误解锁/重启/两种锁定策略：通过或失败
P2：默认关闭/开启/第二个 shell/再次关闭/身份丢失恢复/服务重启：分别通过、失败或未测
P3：换钥阻断/新旧指纹/重复阻断/显式恢复/重启：通过或失败
清理：旧遗留目录、本次 fixture/身份/目录、服务恢复：完成或未完成
失败：步骤号、错误码、简短现象
```

用 `winver` 查看 Windows 版本；Windows 设置的“已安装的应用”查看 Microsoft Edge WebView2 Runtime 版本；第 1 步普通 PowerShell 执行 `& "$env:WINDIR\System32\OpenSSH\ssh.exe" -V` 查看 OpenSSH 客户端版本。服务端版本已写在 `SecretDirectory\server-version.txt`，可以只回报版本字符串。

不要回传密钥、密码、口令、Vault/备份文件或终端正文。不需要给完整桌面截图；如有错误弹窗，文字或仅包含非敏感错误信息的截图即可。

**7. 后续实机反馈已通过；交付证据仍有缺口**

| 部分 | 当前记录 |
| --- | --- |
| 加密备份迁移 | 跨机导入、合并/替换/取消、信任冲突与文件引用失效的实机功能范围，按维护者“实机测试全部通过”反馈登记通过；未新增数据文件取证。 |
| 其余桌面安全与交互 | 原生安全确认、私钥/Agent、主机/分组、锁定/改密、中文输入/焦点/语言/主题等剩余功能范围登记为维护者报告通过；逐项原始记录未保存。 |
| 实际安装与恢复 | 两版本安装升级/卸载保留有既有自动化报告；剩余安装/便携/恢复功能手测登记通过。没有新的环境记录，不推断干净 OS/新用户 VM 或 ARM64。 |
| M0/M1 可见基线 | 维护者报告测试通过，但原始报告和三轮性能数据未保存；任务契约要求的固定可见本地/SSH 性能比较仍缺交付物。有效 ConPTY 包/来源及原生 WSL 入口等 M0 工程缺口不由实机反馈代替。 |
| 最终结论 | 功能与实机测试通过；完整 M1 仍因原始证据及 M0 前置工程交付缺口保持 blocked。未修改标准或 PLAN，未发布、提交或推送，不要求重复已报告手测。 |

旧 RSA 依赖阻断已由实现替换关闭；自动化结果与维护者反馈分别记录。当前没有新增待重测的功能项；缺失的性能数据、运行时包材料和 M0 工程项继续如实保留，不以手测口头结果补写实现或原始测量，也不增加购置硬件的要求。
