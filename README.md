# TENUKI

TENUKI is a XUnity Auto Translator tool built in Rust and powered by a local LLM.


あと、クイックスタートにセットアップ待ちを入れるなら、こっちの方が少し正確です。

```md
## Quick Start

**English**
1. Download the latest release
2. Run `TENUKI.exe` and wait for setup to finish
3. Start your game

**日本語**
1. 最新版をダウンロード
2. `TENUKI.exe` を起動してセットアップ完了まで待つ
3. ゲームを開始

**简体中文**
1. 下载最新版本
2. 运行 `TENUKI.exe` 并等待安装完成
3. 启动游戏

## XUnity Auto Translator

```ini
[Service]
Endpoint=CustomTranslate
FallbackEndpoint=

[Custom]
Url=http://127.0.0.1:14371
EnableShortDelay=False
DisableSpamChecks=True
