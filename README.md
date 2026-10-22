# nekomata

[felis](https://git.natsukium.com/natsukium/felis) のセッションに、タブと分割ペインを備えたネイティブGUIを提供します。
Rust / egui でウィンドウを描画し、felis の公開クライアントライブラリを通じて画面差分を受信します。シェル・PTY・端末の解析・スクロールバックは felis デーモンが管理します。

## 起動

Nix と flakes を有効にした環境で実行します。

```sh
nix run .
```

ビルドしてから起動する場合:

```sh
nix build
./result/bin/nekomata
```

felis デーモンも Nix パッケージに含まれ、初回接続時に自動起動します。GUIとデーモンのソースリビジョンは `1761d2a035d6ccf26da004f395e38b93da2f5686` に固定しています。

flake の対象は `aarch64-darwin`、`x86_64-linux`、`aarch64-linux` です。開発時のビルド・実デーモン接続は `aarch64-darwin` で検証しています。Linux のGUIには X11 または Wayland と OpenGL が必要です。

## 操作

| 操作 | macOS | Linux |
| --- | --- | --- |
| 新しいタブ | Cmd+T | Ctrl+Shift+T |
| 左右に分割 | Cmd+D | Ctrl+Shift+D |
| 上下に分割 | Cmd+E | Ctrl+Shift+E |
| 次のペインに移動 | Cmd+N | Ctrl+Shift+N |
| ペインを閉じる | Cmd+W | Ctrl+Shift+W |
| 前 / 次のタブ | Cmd+← / → | Ctrl+Shift+← / → |
| 選択をコピー | Cmd+C | Ctrl+Shift+C |
| 貼り付け | Cmd+V | Ctrl+Shift+V |

ツールバーからもタブ追加、縦横分割、ペイン・タブの閉鎖、再接続ができます。

- ペインをクリックして選択し、分割バーをドラッグしてサイズを変更します。
- ドラッグで文字を選択します。TUIがマウスを利用している場合は Shift+ドラッグで選択します。
- ホイール・トラックパッドで履歴をスクロールします。マウス入力を要求するTUIにはホイール入力を送ります。
- `−` / `+` ボタンで文字サイズを変更します。
- ウィンドウ・ペイン・タブを閉じても、シェルと実行中のコマンドはデーモン内に残ります。シェル自体を終了するには `exit` などを使います。

## セッションへの再接続

ステータスバーのセッションIDをクリックすると、完全なIDをコピーできます。`Attach…` でそのIDを入力すると、新しいタブで再接続します。

```sh
nix run . -- --attach SESSION_ID
```

felis CLI も開発環境内で使えます。

```sh
nix develop --command felis sessions list
nix develop --command felis sessions kill SESSION_ID
```

`FELIS_SOCKET` を指定すると、通常とは別のローカルデーモンに接続できます。

## 開発・検証

```sh
nix develop
cargo run
cargo test --locked
cargo clippy --locked --all-targets -- -D warnings
cargo fmt --check
bash scripts/test-live.sh
```

`scripts/test-live.sh` は専用ソケットのデーモンを使い、実際の入力・カラー表示・リサイズ・切断後の再接続を検証します。終了時にはテスト専用のデーモンとセッションを片付けます。

```sh
nix flake check
nix fmt -- flake.nix
```

## 初版の対応範囲

テキスト、256色 / RGB、CJK文字、選択・コピー・貼り付け、スクロールバック、名前付きキー、マウス入力、IME確定文字の送信に対応しています。macOSではシステムの日本語フォントをフォールバックとして読み込みます。別のフォントを使う場合は `NEKOMATA_FONT=/path/to/font.ttf` を指定できます。Linuxでは Noto CJK の標準パスも探します。

このGUIは独自のテキスト描画を使います。felis本体の全描画機能を引き継ぐものではありません。Kitty画像、OSC 66文字サイズ、リガチャ、動的パレット変更、全種類の文字装飾、キー解放イベント、OSC 52クリップボード操作、SSH接続・外部からのセッション切替指示には未対応です。デーモン切断後の自動再接続と、タブ・分割配置の保存も未実装です。

IMEやOSクリップボードの操作、TUIのマウス操作、GUIの見た目は実機での追加確認が必要です。

## ライセンス

Apache-2.0。端末エンジンには [felis](https://git.natsukium.com/natsukium/felis)（Apache-2.0）を利用しています。上流を変更せず、固定したGit依存として参照しています。
