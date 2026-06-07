# TENUKI - Release Notes

---

## Version 1.2.5

- **Model Change**: Changed the default model to HY-MT2.
- **Background Text**: As the system has become slightly more instruction-following capable, a background text field has been added to TOML (now supporting styles such as martial arts fantasy).
- **Batch Removal**: Removed batch processing because it had settings that wasted VRAM without actually functioning.

### Version 1.2.0
- **Dictionary system overhaul**
 Changed to register entries containing variables (Attack +5%) in a separate dictionary.

- **Translation system overhaul**
 Changed so that entries with variables can be registered in the dictionary and matched accordingly. Currently limiting the scope to avoid adverse effects and still experimenting.

- **List mode**
 A mode that takes text data and produces a translation TXT. Not very useful, so don't worry about it.

- **Bug fix**
 Fixed an issue where XUnity was sending already translated text in requests, causing infinite proliferation of registered entries.　
  
### Version 1.1.4
- **Bug Fixes** 
  Fixed issues that had been causing slowdo
### Version 1.1.5
  ChatGPT「This time, it's the one that actually made progress.」
  
### Version 1.1.4
- **Bug Fixes** 
  Fixed issues that had been causing slowdowns and cases where processing was skipped. The major restructuring has also mostly settled down, and it finally feels ready for people to use with confidence.

### Version 1.1.3
* **Setup Stabilization:** 
　Fixed an issue where the creation process for new installations was completely left out because I was frantically trying to ensure safe migrations from older versions.

### Version 1.1.2
* **Setup Stabilization:** 
　Almost entirely replaced the processing structure to stabilize it. As a result, it ended up completely broken.

### Version 1.1.1

- **Improved Setup Stability**
  Enhanced resilience to various network conditions.
  Significantly overhauled the startup process to ensure the application launches reliably with a single double-click. 
  This update is practically a complete rebuild.
	
## Version 1.1.0
- **Setup Overhaul**  
  Initial setup has been fully automated.  
  Pushing automation too far can create its own problems, including the risk of being treated like suspicious software, but the ideal experience is still simple: double-click once and it just works, and when you no longer need it, you can throw the whole folder into the trash.  
  It was a huge amount of work, but I rebuilt the setup process from scratch to make that possible.

- **Arabic Support**  
  Arabic support had been quietly added behind the scenes. I spent a lot of time thinking about right-to-left handling and related processing, but since patches already seem to exist for some environments, I decided it was still worth exposing Arabic as a selectable language for people who are fine with direct left-to-right translated output.  
  Line-wrapping insertion is now disabled by default for this case.

## Version 1.0.0e
- **VULKAN**  
  Fixed an issue where some VULKAN configurations could end up not using the GPU at all.  
  I had been stuck running on CPU myself, and once it started working again, I assumed the code had already been fixed too, so I did not realize the problem was still there.

## Version 1.0.0b
- **Expression Quality**  
  Changed the model settings from an overly restrictive configuration to the official standard settings.  
  Long-form output improved so noticeably that it became painfully obvious the awkward phrasing had been my fault.

## Version 1.0.0
- **External Input Support**  
  Added support for receiving input from external connections.  
  This makes it possible to run TENUKI on a PC that is not actually running the game.

- **Structural Processing**  
  Reworked the dictionary registration logic to better handle word order changes, long-context translation, and compatibility with other games.  
  This reduced hit rate, reduced editability, and lowered dictionary compatibility, but improved contextual flow and number sequence handling.  
  In short, I spent four days thinking through how to rebuild it almost from scratch.

- **Long Text Wrapping**  
  Added a workaround for cases where the source text is short but the translated result becomes too long and overflows the screen.  
  The threshold values are basically educated guesses, so enabling this may break layout in some games.  
  I still cannot really read English, but at least the text now stays inside the box.

## Version 0.9.1
- **Display Cleanup**  
  Reorganized the three-layer logging view and cleaned up features such as the unused source-language selector.

- **Structural Processing**  
  Added and adjusted missing processing that turned up while testing with English and other games.

## Version 0.9.0
- **Custom Languages**  
  Added a language code field and custom language name input to language switching.  
  Depending on the model, it might actually translate into that language.

- **Structural Processing**  
  Improved handling for source and translated text in multilingual cases where support was still incomplete.  
  As a side effect, translation quality dropped slightly and is still being tuned, but overall speed improved.  
  Honestly, I had not preserved the old implementation properly, so once I lost track of what it had been doing, rebuilding it ended up making it faster.

## Version 0.8.6
- **Multilingual Support**  
  Added support for selecting both source and target languages.  
  A lot of combinations are still untested, so consider it lucky if it works perfectly.

- **Dictionary System Cleanup**  
  Moved dictionaries into the `dicts` folder inside the TENUKI directory to support Normal Mode.

## Version 0.8.5
> *A few small features and changes, plus a large number of bug fixes. I do not want to remember this one.*

## Version 0.8.1
- **Normal Mode**  
  Added a passthrough mode that simply returns the translation without extra structural handling.

## Version 0.7.4
- **Dictionary Entry Count Display Fix**
  - Fixed an issue where the dictionary entry count was displayed as 0
  - Corrected the display so loaded entries and newly registered entries are both counted
  - Fixed the count after language switching so each language now shows the correct dictionary size

## Version 0.7.3
- **Reduced Dictionary Load Logs**  
  Suppressed dictionary loading logs during startup and restart.

## Version 0.7.2
- After receiving a report that TENUKI might always be referencing the Japanese dictionary, I investigated and fixed the issue.
- I also found that `。` had been registered as an invalid dictionary entry, so I removed it in an attempt to improve the overall state.

## Version 0.7.1
- After receiving a report that English switching was not working, I investigated and found that the code already existed but was not actually being used, so I implemented it properly.
- I also realized that TENUKI’s existing restart button could be reused for language switching, so the feature was changed to use that method for Japanese/English switching.

## Version 0.7.0
- **Initial Release**






# TENUKI - リリースノート

---

### Version 1.2.5
  モデル変更　　DefaultのモデルをHY-MT2へ変更。
  背景テキスト　指示が少しできるようになったのに伴い、バックグラウンドテキスト欄をTOMLに追加（武侠風などが可能に）
  バッチ廃止　　実際に機能が働いてないのに設定だけありVRAMを無駄にしていたので廃止。


### Version 1.2.0
  **辞書システム改修**　　　
  攻撃+5％など変数があるものを別辞書にまとめて登録するように変更
  
  翻訳システム改修　
  変数があるものをまとめて辞書登録と辞書ヒットできるように変更。多少狭くして悪い影響が出ない範囲にまだ絞って試してる。
  
  リストモード
  テキストデータを流し込んで翻訳TXTを作るモード。あんまり使い道ないから気にしないで。
  
  バグフィックス
  XUnityが翻訳後のものをリクエストで送ってくるのを登録して無限増殖していたのに対応

### バージョン 1.1.4　
* **不具合修正**　　　　速度低下してたものや処理抜けしてたものを修正。ほぼ再構成も一段落してやっと人が使うの安心できる。

### バージョン 1.1.3　
* **セットアップ安定化**　　古いバージョンから安全に移行できるように必死になってたら新規の場合の作成が抜けてるという状態になってたのを修正

### バージョン 1.1.2　
* **セットアップ安定化**　　処理構造を安定化するためほぼ入替。結果動かないものになってた。

### バージョン 1.1.1　
* **セットアップ安定化**　　回線状況への対応を強化　ダブルクリックで起動まで到達できるように色々やった。ほぼ作り直し。

### バージョン 1.1.0　
* **セットアップ変更**　　初期セットアップを完全に自動化。　あんまりやるとウイルス扱いされやすいとか色々問題あるけど、最初にダブルクリックしたらもう使える・いらなくなったらフォルダごとゴミ箱に捨てれるのが一番いいので、ほんとに大変だったけど新規に作り直した。
* **アラビア語対応**　　　こっそり対応してて、右書きとかの処理とかいろいろ考えたが、もうパッチがあるようなので左書きでそのまま翻訳されるだけでいい人がいると判断したので選択言語化。　折り返しの改行処理をデフォルトでオフにするように設定。

### バージョン 1.0.0e　
* **VULKAN**　    VULKAN環境でGPUが使われない設定になり得た問題を修正。　自分もCPUだけになって困っていたのに、直ったからコードも直ったと思っていて問題あることに気づいてなかった。
 　
### バージョン 1.0.0ｂ　　
* **表現力**　　　　モデルの設定が表現力極力抑えた設定になっていたのを公式の標準に変更。　あからさまに長文良くなって私のせいであんな文章だったことが明らかに・・・・・

### バージョン 1.0.0　　　
* **外部接続**　　　外部接続からの入力を受けれる設定を追加。　ゲームしてないPCでの動作が可能に
* **構造処理**     辞書登録ロジックを、語順変化と長文の文脈、ほかのゲームに対応するため変更。　hit率低下編集性低下辞書互換性低下、文脈向上、数字配列適正化　　　要は、まるまる新規化するのに4日考えた　　
* **長文折り返し**　　元言語が短い場合に、翻訳後長文が画面から飛び出る現象への対処を導入。　山勘で数字入れてるので、設定を使うと崩れるゲームも多そう。　　　　英語読めないけど、文字は収まるようになったよ。

### バージョン 0.9.1
* **表示整理**　　　ログの3層化や使われていなかった原語選択などを整理。
* **構造処理** 　　　英語やその他のゲームで試してみたら抜けていた処理を追加変更。

### バージョン 0.9.0 
* **カスタム言語** 　言語切り替えに、国コードと言語入力欄を追加。　モデル次第で翻訳するかも？
* **構造処理** 　　原語と翻訳を、多言語での対応未整備部分の強化変更。また結果的に翻訳精度は若干落ちて調整中だが基本速度向上。（古いものをちゃんと保存とかしてないので、元の処理がわからなくなって、作り直したら速くなっただけです…）

### バージョン 0.8.6
* **多言語化実装:** 原語と翻訳を選択できるように。未検証多数のため動いたらラッキーだと思ってね。
* **辞書システム整理:** ノーマルモード対応のためTENUKIフォルダのdictsフォルダに置くことに変更。

### バージョン 0.8.5
> *少しの新機能と変更、多数の不具合を調整。思い出したくない。*

### バージョン 0.8.1
* **ノーマルモード実装:** パススルーでなにもないで翻訳だけ返す。

### バージョン 0.7.4
**辞書エントリ数の表示を修正:**
* 辞書エントリ数が 0 と表示される問題を修正
* 読み込まれたエントリ + 新規登録エントリが正しく表示されるように修正
* 言語切り替え後に各言語の辞書数が適切に表示されるよう修正

### バージョン 0.7.3
* 起動時 / 再起動時の辞書読み込みログを抑制

### バージョン 0.7.2
* DeepSeek から辞書が常に日本語を参照している可能性があるとの指摘を受け、確認・修正を実施。
* また、「。」が辞書に不正なエントリとして登録されていたため、これを削除することで状態が改善すると考え、実装を行った。

### バージョン 0.7.1
* 英語切り替え機能が動作しないとの報告を受け、調査したところコード自体は存在するが使用されていなかったため、実装した。
* 言語切り替えとして TENUKI ボタンの再起動をそのまま利用できることに気づき、その方式に変更（日本語/英語表示切り替え）。

### バージョン 0.7.0
* 初版リリース