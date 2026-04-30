# TENUKI

TENUKI is a XUnity Auto Translator tool built in Rust and powered by a local LLM.

## Quick Start

## English

1. Download the latest release.
2. Run `TENUKI.exe`.  
   TENUKI will automatically download and set up `HY-MT1.5-1.8B-Q6_K.gguf` and the llama.cpp runtime that matches your backend.  
   Windows SmartScreen may show a warning.
3. When setup is finished, select the language you want to translate into. TENUKI will then automatically translate your game text.

## 日本語

1. 最新版をダウンロードします。
2. `TENUKI.exe` を起動します。  
   `HY-MT1.5-1.8B-Q6_K.gguf` と、バックエンドに合った llama.cpp runtime を自動でダウンロードしてセットアップします。  
   （Windows の SmartScreen が反応する場合があります。）
3. セットアップが終わったら、翻訳したい言語を選択します。以後、ゲームのテキストを自動で翻訳します。

## 简体中文

1. 下载最新版本。
2. 运行 `TENUKI.exe`。  
   TENUKI 会自动下载并设置 `HY-MT1.5-1.8B-Q6_K.gguf` 以及适合当前后端的 llama.cpp runtime。  
   Windows SmartScreen 可能会显示警告。
3. 设置完成后，选择想要翻译成的语言。之后，TENUKI 会自动翻译游戏文本。


   
```ini
[Service]
Endpoint=CustomTranslate
FallbackEndpoint=

[Behaviour]
TemplateAllNumberAway=True

[Custom]
Url=http://127.0.0.1:14371
EnableShortDelay=False
DisableSpamChecks=True
