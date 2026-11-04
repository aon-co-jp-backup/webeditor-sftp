# 開発方針＆開発環境ルール(webeditor-sftp)

作業ドライブは`F:\`。本リポジトリのローカルcloneは`F:\webeditor-sftp`
(サブディレクトリ無し)。全リポジトリ共通ルール(参照資料一覧・AI駆動開発
ツールに関する所感・確認不要の自動継続/リミット解除後の自動再開・
白画面バグ等を見逃さない検証徹底、等)は
[`open-raid-z`](https://github.com/aon-co-jp/open-raid-z)の`CLAUDE.md`を
正本とし、詳細をここに複製せず参照すること。

## このリポジトリの役割

ブログ感覚のWEB文章編集エディターと、秘密鍵・公開鍵を安全に受け渡し
できるSFTPツールをハイブリッド統合したクロスプラットフォームアプリ。
役割の全体像は[`README.md`](README.md)を参照。

## アーキテクチャ概要

Rust側はCargoワークスペース(`Cargo.toml`)として3クレートに分割
(2026-09-30〜、VSCode拡張機能でのサイドカー再利用を見据えた構成):

```
webeditor-sftp/
  Cargo.toml          [workspace] members = [core, cli, src-tauri]
  core/               webeditor-core(ライブラリ、Tauriに非依存)
    src/
      editor/          ブログ⇔コードHTML相互変換
      sftp/            鍵ペア生成・セキュアストレージ・SFTP接続
  cli/                webeditor-sidecar(バイナリ)
    src/main.rs         VSCode拡張機能向けサイドカーCLI。
                        stdin/stdoutでJSON Linesプロトコルを話す
                        (`{"id","method","params"}`→
                        `{"id","ok","result"|"error"}`)。
  src-tauri/          デスクトップアプリ本体(Tauriバックエンド)
    src/
      commands.rs       `webeditor_core`への薄い#[tauri::command]ラッパー
      lib.rs            invoke_handlerへの登録のみ
    tauri.conf.json
  src/                フロントエンド(Web技術、CodeMirror 6ベース)
    editor/             ブログモード(タグ非表示)/コーディングモード切替
    sftp-ui/            鍵の生成・インポート・エクスポート・接続UI
    index.html
  PORTING.md          複数セッション横断の到達点・次回再開ポイント
```

**実装は必ず`core/`に書く。** `src-tauri/src/commands.rs`と
`cli/src/main.rs`はどちらも`webeditor_core`の薄いラッパーに徹し、
ロジックの複製は行わない(Windows版のVSCode拡張機能からは`cli`の
サイドカーバイナリを`child_process.spawn`で呼び出す想定、
PORTING.md「2026-09-30」節参照)。

### プラットフォーム対応方針

- Windows / Linux(WindowsにインストールされたLinux含む) / (将来)macOS:
  Tauriデスクトップビルドで対応。
- Android / タブレット: Tauri 2.0 Androidターゲット
  (`x86_64-linux-android`等、本機は既に`rustup target`導入済み)。
- (将来購入予定)iPhone / iPad: Tauri 2.0 iOSターゲット。Xcode環境が
  必要なため、macOS実機/CI入手まで実装は保留。

### 鍵の安全な受け渡し(最重要方針)

- 秘密鍵は生成後、平文でアプリ外(クリップボード履歴・一時ファイル等)
  に残さない設計とする。
- 保存先は各OSのセキュアストレージAPI経由。実装は`keyring`クレート
  (Windows: Credential Manager / macOS: Keychain / Linux: Secret
  Service)。`src-tauri/src/sftp/keystore.rs`参照。
  **⚠️重要**: `keyring`クレート3.x系はOS別バックエンドfeature
  (`windows-native`/`apple-native`/`linux-native-sync-persistent`)を
  `Cargo.toml`の`[target.'cfg(...)'.dependencies]`で明示的に有効化
  しないと、書き込みがエラーを返さず無音で失敗する(2026-09-29に
  実機検証で発覚・修正済み、詳細はPORTING.md参照)。新しいOSターゲット
  を追加する際は、対応するfeatureを必ず追加すること。Android向けの
  ネイティブバックエンドはkeyringクレートに存在しないため、Android
  実機対応時は別のセキュアストレージ手段を要検討。
- 鍵ペア生成はed25519(`russh-keys`、`src-tauri/src/sftp/keys.rs`)。
- SFTP接続は`russh`+`russh-sftp`(純Rust実装、libssh2/OpenSSLへの
  ネイティブ依存なし、`src-tauri/src/sftp/client.rs`)。
  ホスト鍵検証はTOFU方式(`src-tauri/src/sftp/known_hosts.rs`)。
  初回接続でフィンガープリントを記録し、以後不一致なら接続を拒否する。
  記録は`~/.ssh/known_hosts`と同様、秘密情報ではないため平文JSON
  (アプリ設定ディレクトリ配下)。
- デバイス間の鍵の受け渡し(例: Windows→Android)は、この端末間で
  Claude等の外部AIツールを経由させない(ユーザー要件)。**QRコード方式
  を採用**(`src-tauri/src/sftp/pairing.rs`)。公開鍵のみをQR化し、
  秘密鍵は画面越しの盗撮リスクがあるため対象外。生成・カメラでの
  読み取り(`jsqr`、`src/main.ts`)の双方向を実装済み。読み取った
  公開鍵は`sftp_append_authorized_key`コマンドでリモートの
  `~/.ssh/authorized_keys`へ追記できる。
- `audiocafe-tokyo-rust`のような`include_str!`コンパイル時埋め込み
  方式のサイト向けに、SFTPアップロード後の任意コマンド実行
  (`exec_after_upload`、`sftp_upload_text`)を実装。SSH execチャネル
  経由でリモートビルド・サービス再起動まで一括実行できる。

### エディターの2モード

1. **ブログモード**: HTML/CSSタグを意識せず文章を書くと、見出し・
   段落・強調等を自動でタグ化。括弧/記号の自動補完付き。
2. **コーディングモード**: 通常のVS Code的な生のHTML+CSS+
   JavaScript/TypeScript/React編集。UTF-8前提、シンタックス
   ハイライト・自動補完付き。

両モードは同じドキュメントに対する見た目の切り替えであり、
コーディングモードで書いたHTMLをブログモードで開いても崩れない
ことを設計上の必須要件とする。

## 対応バックエンド(サーバー側プログラミング言語)

エディターがシンタックスハイライト/自動補完で意識する対象:
Rust+Axum/Poem/RPoem、PHP+Laravel、Python+FastAPI。

## JSON処理の使用方針: `parse_secure_sonic`(速い型+安全)と`sonic-rs`は使える所では全て使う(2026-10-14、ユーザー指示)
正本は[`RS-JSON`](https://github.com/aon-co-jp/RS-JSON)(`README.md`・`PORTING.md`)。

**方針**: JSONを読む所は、使える所では**全て** `rust-json`(RS-JSON)の `parse_secure_sonic`(`fast`機能、結果は`sonic_rs::Value`)を使う。
`sonic-rs` を直接使える所(型付き構造体への読み込みなど)も同様。使えない所は `parse_secure`(結果は`serde_json::Value`)を使う。
外部から来る信頼できない入力(HTTP本文・ネットワーク越しのファイル・アップロード)は、**必ずサイズ・深さ・重複キーの上限つき**で読む。

**実測(7 MB・4万件)**: `serde_json` 200 ms / `parse_secure` 217 ms / **`parse_secure_sonic` 83 ms(約2.4倍速い、防御込み)** / `sonic-rs`そのまま 33 ms(約6倍、防御なし)。
`serde_json::Value`に読む限りSIMDでもほぼ速くならない(188 ms)。**速さの源は、結果を連続した領域に詰め込む値の型**。

**使える所 / 使えない所**
- ✅ 読み取り中心で、JSONを大きく・頻繁に読む所(キャッシュJSON、設定、APIの応答、ログ、インデックス)。
- ✅ 外部からの入力の入口(速さより防御が目的。`parse_secure`でも可)。
- ⚠️ `serde_json::Value`を関数間で受け渡している既存コードは、型を`sonic_rs::Value`に直す書き換えが要る(`JsonValueTrait`/`JsonContainerTrait`のメソッド)。書き換えずに`serde_json::Value`へ読むと速くならない。
- ❌ WASM(`rust_json::light`、依存ゼロ)、PHP/JavaScript側のJSON処理(Rustの部品は使えない)。
- 注意: 依存先の`sonic-rs`は内部で`unsafe`を使う(自前コードに`unsafe`は無い)。小さなJSON(数KB)では体感差が出ない。速度目的の書き換えは、実測で効く所から。

**導入**: `Cargo.toml`に `rust-json = { path = "../RS-JSON", features = ["fast"] }`。使用例は`RS-JSON/README.md`。

**このリポジトリでの適用状況**: 未適用(方針のみ)。コードの切り替えは、JSONを大きく・頻繁に読む所から実測して順に行う。

## JSON handling policy: use `parse_secure_sonic` (fast type + secure) and `sonic-rs` everywhere they can be used (2026-10-14, user instruction)
The source of truth is [`RS-JSON`](https://github.com/aon-co-jp/RS-JSON) (`README.md`, `PORTING.md`).

**Policy**: wherever JSON is read, use `parse_secure_sonic` from `rust-json` (RS-JSON, the `fast` feature; the result is `sonic_rs::Value`) **everywhere it can be used**.
Use `sonic-rs` directly where possible too (e.g. reading into typed structs). Where it cannot be used, use `parse_secure` (result: `serde_json::Value`).
Untrusted input from outside (HTTP bodies, files over the network, uploads) **must always be read with size / depth / duplicate-key limits**.

**Measured (7 MB, 40,000 records)**: `serde_json` 200 ms / `parse_secure` 217 ms / **`parse_secure_sonic` 83 ms (about 2.4x faster, with the defences)** / raw `sonic-rs` 33 ms (about 6x, no defences).
Reading into `serde_json::Value` gives almost no speed-up even with SIMD (188 ms). **The speed comes from the value type that packs the result into contiguous memory.**

**Where it can / cannot be used**
- ✅ Read-heavy code that parses large or frequent JSON (cache JSON, config, API responses, logs, indexes).
- ✅ Entrances for outside input (the goal there is defence rather than speed; `parse_secure` is fine).
- ⚠️ Existing code that passes `serde_json::Value` between functions needs its type changed to `sonic_rs::Value` (methods from `JsonValueTrait` / `JsonContainerTrait`). Reading into `serde_json::Value` without that change is not faster.
- ❌ WASM (`rust_json::light`, zero dependencies) and the JSON handling on the PHP / JavaScript side (Rust components cannot be used there).
- Note: the `sonic-rs` dependency uses `unsafe` internally (our own code has none). For small JSON (a few KB) the difference is not noticeable; do speed-motivated rewrites where measurements show a benefit.

**Adoption**: in `Cargo.toml`, `rust-json = { path = "../RS-JSON", features = ["fast"] }`. Example in `RS-JSON/README.md`.

**Status in this repository**: Not yet applied (policy only). Code will be switched over place by place, starting where measurements show large or frequent JSON reads.

## API 方針: REST 全廃・gRPC / Connect へ (2026-11-04)

### 日本語
- **REST は全廃する。新しい API は REST で作らない。** ブラウザや `curl` から呼ぶ API は **Connect の単純呼び出し**(`POST /<パッケージ>.<Service>/<Method>`、`content-type: application/json`、本文は JSON)、サービス間は **gRPC / gRPC-web**(RPoem の管理API `open_runo.admin.v1.Admin` と同じ流儀)。ストリーミングは、必要になってから。
- **既存の REST は、触るときに順に移す。** 移したら旧 REST の経路は消す。移行が終わるまで残るが、**新規には追加しない**。
- **JSON は RS-JSON に統一**する(読み込みは `from_str_secure` / `parse_secure` でサイズ・深さ・重複キーの上限つき、書き出しは `rust_json::fast`)。ブラウザの WASM に `prost` は入れない(サイズが増えるため)。
- **例外(自分たちの API ではなく、相手の規格が決めているもの)**: ACME、OAuth/OIDC のコールバック、第三者(GitHub など)からの Webhook、ヘルスチェック、静的ファイル配信、OTLP など。これらは規格どおりの HTTP を受ける。
- 参考実装: `RPoem`(`open_runo.admin.v1.Admin`)、`open-easy-web`(`server/src/connect.rs`、`proto/open_easy_web/admin/v1/admin.proto`)。
- ※ このリポジトリのコードが既に方針どおりかどうかは、各リポジトリの現状による。この節は方針の記録であり、コードの変更を意味しない。

### English
- **REST is being abolished. New APIs are not built as REST.** APIs called from browsers or `curl` use **Connect unary calls** (`POST /<package>.<Service>/<Method>`, `content-type: application/json`, JSON body); service-to-service calls use **gRPC / gRPC-web** (the same style as RPoem's admin API `open_runo.admin.v1.Admin`). Streaming only when it is actually needed.
- **Existing REST endpoints are migrated one by one, when they are touched.** The old REST route is removed once migrated. Until migration is finished they remain, but **no new ones are added**.
- **JSON is unified on RS-JSON**: reading with `from_str_secure` / `parse_secure` (size, depth and duplicate-key limits), writing with `rust_json::fast`. `prost` is not put into browser WASM (it would grow the binary).
- **Exceptions (shapes dictated by someone else's standard, not our own API)**: ACME, OAuth/OIDC callbacks, webhooks from third parties (e.g. GitHub), health checks, static file serving, OTLP, and so on. These accept plain HTTP as the standard requires.
- Reference implementations: `RPoem` (`open_runo.admin.v1.Admin`) and `open-easy-web` (`server/src/connect.rs`, `proto/open_easy_web/admin/v1/admin.proto`).
- Note: whether the code in a given repository already follows this policy depends on that repository. This section records the policy; it does not mean the code has been changed.

### Deutsch (KI-Übersetzung, redaktionell überarbeitet; noch nicht von Muttersprachlern geprüft)
- **REST wird vollständig abgeschafft; neue APIs entstehen nicht mehr als REST.** APIs, die aus dem Browser oder per `curl` aufgerufen werden, verwenden **unäre Connect-Aufrufe** (`POST /<Paket>.<Service>/<Methode>`, `content-type: application/json`, JSON-Body). Zwischen Diensten kommen **gRPC bzw. gRPC-web** zum Einsatz, im selben Stil wie die Admin-API von RPoem (`open_runo.admin.v1.Admin`). Streaming wird erst ergänzt, wenn es tatsächlich gebraucht wird.
- **Bestehende REST-Endpunkte werden schrittweise umgestellt, und zwar immer dann, wenn man ohnehin an ihnen arbeitet.** Sobald ein Endpunkt umgestellt ist, wird die alte REST-Route entfernt. Bis zum Abschluss der Migration bleiben die übrigen bestehen, es werden aber **keine neuen mehr hinzugefügt**.
- **JSON wird einheitlich über RS-JSON verarbeitet**: Gelesen wird mit `from_str_secure` bzw. `parse_secure` (mit Obergrenzen für Größe, Verschachtelungstiefe und doppelte Schlüssel), geschrieben mit `rust_json::fast`. `prost` wird nicht in das Browser-WASM aufgenommen, da es die Datei vergrößern würde.
- **Ausnahmen (Schnittstellen, deren Form ein fremder Standard vorgibt und die nicht zu unserer eigenen API gehören)**: ACME, OAuth/OIDC-Callbacks, Webhooks von Drittanbietern (z. B. GitHub), Health-Checks, die Auslieferung statischer Dateien, OTLP und Ähnliches. Diese nehmen, wie vom jeweiligen Standard verlangt, gewöhnliches HTTP entgegen.
- Referenzimplementierungen: `RPoem` (`open_runo.admin.v1.Admin`) und `open-easy-web` (`server/src/connect.rs`, `proto/open_easy_web/admin/v1/admin.proto`).
- Hinweis: Ob der Code eines einzelnen Repositorys diese Richtlinie bereits erfüllt, hängt vom jeweiligen Repository ab. Dieser Abschnitt dokumentiert lediglich die Richtlinie; er besagt nicht, dass der Code schon angepasst wurde.

### Русский (перевод выполнен с помощью ИИ и отредактирован; носителями языка пока не проверен)
- **От REST мы полностью отказываемся; новые API больше не создаются в стиле REST.** API, к которым обращаются из браузера или через `curl`, используют **унарные вызовы Connect** (`POST /<пакет>.<Service>/<Method>`, `content-type: application/json`, тело запроса — JSON). Для взаимодействия между сервисами применяются **gRPC и gRPC-web** — в том же стиле, что и административный API RPoem (`open_runo.admin.v1.Admin`). Потоковая передача данных добавляется лишь тогда, когда в ней действительно возникает необходимость.
- **Существующие REST-эндпоинты переводятся поэтапно — по мере того, как с ними приходится работать.** После перевода старый REST-маршрут удаляется. Пока миграция не завершена, оставшиеся эндпоинты сохраняются, но **новые не добавляются**.
- **Работа с JSON унифицирована на основе RS-JSON**: чтение — через `from_str_secure` или `parse_secure` (с ограничениями на размер, глубину вложенности и повторяющиеся ключи), запись — через `rust_json::fast`. `prost` в браузерный WASM не включается, поскольку это увеличило бы размер файла.
- **Исключения (интерфейсы, форму которых определяет чужой стандарт, а не наш собственный API)**: ACME, обратные вызовы OAuth/OIDC, вебхуки сторонних сервисов (например, GitHub), проверки работоспособности, раздача статических файлов, OTLP и тому подобное. Такие интерфейсы принимают обычный HTTP, как того требует соответствующий стандарт.
- Эталонные реализации: `RPoem` (`open_runo.admin.v1.Admin`) и `open-easy-web` (`server/src/connect.rs`, `proto/open_easy_web/admin/v1/admin.proto`).
- Примечание: соответствует ли код конкретного репозитория этой политике, зависит от самого репозитория. Данный раздел лишь фиксирует политику и не означает, что код уже изменён.
