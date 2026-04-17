# TENUKI

TENUKI is a XUnity Auto Translator tool built in Rust and powered by a local LLM.

## Current Version

**1.1.0**

## What It Is

TENUKI receives text from external inputs such as XUnity, preprocesses it, sends it to `llama-server`, stores the local LLM translation result in its dictionary, and returns the response.

Its main flow is as follows:

1. Receive a request from an external source
2. Prepare the input through structural processing or passthrough mode
3. Check the dictionary and cache
4. Send only unresolved parts to the local LLM through `llama-server`
5. Register the translation result in the dictionary, postprocess it, and return it

## Structure

This project is broadly divided into the following roles.

### 1. Main Runtime
- `src/main.rs`
- `src/backend/`
- `src/ui/`

In normal mode, the GUI, core backend, translation server, dictionary handling, and model communication are handled here. If the runtime and model are already available, the application enters normal mode immediately after launch.

## Endpoint

XUnity Auto Translator `Config.ini`

```ini
[Service]
Endpoint=CustomTranslate
FallbackEndpoint=

[Custom]
Url=http://127.0.0.1:14371
EnableShortDelay=False
DisableSpamChecks=True


- `GET /translate?text=...` : text translation
- `POST /translate` : text translation

### 2. Launcher
- `src/launcher/`

Handles initial setup, repair, and pre-launch checks.

### 3. Separated Configuration
- `config.toml`
- `launcher_config.toml`

`config.toml` stores translation behavior, UI settings, and TENUKI server settings.  
`launcher_config.toml` stores backend, model, and `llama-server` startup conditions as launcher-specific settings.

### 4. Translation Modes
- `structural`
- `passthrough`

In game mode, TENUKI protects structural boundaries while extracting visible text for translation.  
In passthrough mode, it sends the input directly to translation as-is.

### Public Contents
This repository is mainly intended to publish source code and documentation files.

- `src/`
- `Cargo.toml`
- `Cargo.lock`
- `README.md`
- `Release Notes.txt`

### Runtime Data
The following are used at runtime and are not intended to be included directly in the public repository.

- `runtime/` : downloaded llama.cpp runtime files
- `models/` : downloaded GGUF model files
- `dicts/` : dictionary data
- `profiles/` : structural processing profiles

## Setup

TENUKI launches after automatic setup with a single double-click.

If the required runtime or model files are missing, the launcher performs the following:

1. Prepare required directories
2. Determine backend candidates from GPU information
3. Download and extract the backend runtime
4. Download the model
5. Download and verify `llama-server`
6. Save `launcher_config.toml`

If `launcher_config.toml` does not exist, it is generated automatically.

## Requirements

- Windows
- Rust
- Cargo

## Build

``bash
cargo build --release



# TENUKI

TENUKI は、Rust で実装されたローカルLLM動作のゲームテキスト翻訳ツールです。

## 現在のバージョン

**1.1.0**

## これは何か

XUnity などの外部入力からテキストを受け取り前処理をして、 `llama-server` に送り、ローカルLLMの翻訳結果を辞書登録してリクエストを返します。

主な流れは次のとおりです。

1. 外部からのリクエストを受け取る
2. 構造処理またはパススルー処理で入力を整える
3. 辞書とキャッシュを確認する
4. 未解決の部分だけを `llama-server` でローカルLLMに送る
5. 翻訳結果を辞書登録、後処理して返却

## 構成

このプロジェクトは、大きく分けて以下の役割に分かれています。

### 1. 通常動作本体
- `src/main.rs`
- `src/backend/`
- `src/ui/`

通常モードでは、GUI、本体バックエンド、翻訳サーバー、辞書処理、モデル通信をここで扱います。ランタイムとモデルが揃っていれば、起動後そのまま通常モードに入ります。

## エンドポイント

TENUKI はローカル翻訳サーバーとして動作します。  
デフォルトのポートは `14371` です。

- `GET /translate?text=...` : テキスト翻訳
- `POST /translate` : テキスト翻訳

### 2. ランチャー
- `src/launcher/`

初回セットアップ、修復、起動前チェックを担当します。


### 3. 設定の分離
- `config.toml`
- `launcher_config.toml`

`config.toml` は翻訳挙動、UI、TENUKI サーバー設定を持ちます。  
`launcher_config.toml` は backend、モデル、`llama-server` 起動条件を持つ、ランチャー専用設定です。

### 4. 翻訳処理のモード
- `structural`
- `passthrough`

ゲームモードでは、構造境界を保護しながら可視テキストを抽出して翻訳します。  
パススルーモードでは、入力をそのまま翻訳に渡します。


### 公開対象
このリポジトリでは、主にソースコードと説明ファイルを公開対象としています。

- `src/`
- `Cargo.toml`
- `Cargo.lock`
- `README.md`
- `Release Notes.txt`

### 実行時に使うもの
以下は実行時に使われるデータであり、公開リポジトリにそのまま含める前提ではありません。

- `runtime/` : ダウンロードされた llama.cpp ランタイム
- `models/` : ダウンロードされた GGUF モデル
- `dicts/` : 辞書データ
- `profiles/` : 構造処理内容

## セットアップ

TENUKI はダブルクリック1回で自動セットアップ後、起動します。

必要な runtime や model が不足している場合、ランチャーが以下を行います。

1. 必要ディレクトリの準備
2. GPU 情報から backend 候補を決定
3. backend runtime の取得と展開
4. model のダウンロード
5. `llama-server` の取得と検証
6. `launcher_config.toml` の保存

`launcher_config.toml` は存在しない場合、自動生成。

## 必要環境

- Windows
- Rust
- Cargo

## ビルド

```bash
cargo build --release
