# nekomata

[felis](https://git.natsukium.com/felis-terminal/felis) のセッションに、タブと分割ペインを備えたネイティブGUIを提供します。
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

macOSでDock / Finderから起動する場合:

```sh
bash scripts/install-macos.sh
open ~/Applications/nekomata.app
```

`~/Applications/nekomata.app` をDockへドラッグすると固定できます。更新時も同じインストールコマンドを使います。Nixの依存を保持するリンクは `~/.local/share/nekomata/package`（`XDG_DATA_HOME`指定時はその配下）に作成します。アプリから起動するとホームディレクトリでシェルを開き、CLIから起動すると元のカレントディレクトリを使います。

felis デーモンも Nix パッケージに含まれ、初回接続時に自動起動します。GUIとデーモンのソースリビジョンは `71f4a5ca11ed9bee3422444c52fc425e88e00a81` に固定しています。[上流のプロンプト再描画修正](https://git.natsukium.com/felis-terminal/felis/pulls/38)を含み、ペインをライブリサイズしても以前の幅のプロンプトが残る問題を解消します。

旧版のデーモンがすでに動いている場合は、既存のシェルを終了してから `nix develop --command felis daemon stop` を実行し、nekomata を起動し直してください。GUIの再起動だけではデーモンのコードは切り替わりません。

Nixパッケージにはfish用の起動フックも含まれます。fishの初期化区間を端末に通知することで、まだコマンドを実行していない最初のプロンプトもライブリサイズ時に描き直せます。ユーザーのfish設定ファイルは変更せず、nekomataから起動するfishだけで有効になります（fish 4以降の `mark-prompt` 機能が必要です）。

flake の対象は `aarch64-darwin`、`x86_64-linux`、`aarch64-linux` です。開発時のビルド・実デーモン接続は `aarch64-darwin` で検証しています。Linux のGUIには X11 または Wayland と OpenGL が必要です。

## 操作

フォントはkitty.confに合わせた **UDEV Gothic NF / 15pt** です。Nixパッケージと開発シェルでフォントを供給します。現在はフォントとキー設定をハードコードしています。

| 操作 | キー |
| --- | --- |
| 新しいタブ | Alt+C |
| 次 / 前のタブ | Alt+T / Alt+Shift+T |
| 上下に分割 | Alt+W → Alt+S |
| 左右に分割 | Alt+W → Alt+V |
| ペインを閉じる（detach） | Alt+W → C |
| 左 / 下 / 上 / 右のペインに移動 | Alt+W → H / J / K / L |
| 文字サイズを初期値に戻す | Cmd+0 |
| 文字サイズを1.1倍 / 1÷1.1倍にする | Cmd+= / Cmd+- |
| コマンドパレット | Alt+Shift+P |
| 選択をコピー | Cmd+C（macOS） / Ctrl+Shift+C（Linux） |
| 貼り付け | Ctrl+Shift+V（Cmd+Vも利用可能） |

`→` はキーを順番に押す操作です。分割時はシェルがOSC 7で通知した現在のローカルディレクトリを引き継ぎます。通知がない場合はnekomataの起動ディレクトリを使います。以前のタブ・分割用ショートカットは置き換えました。設定ファイルがまだないため、kittyの `Alt+Shift+R`（設定再読み込み）は未割り当てです。

上部は `≡ | tab1 | tab2 | …` の1行構成です。左端の `≡` メニューからタブ追加、縦横分割、ペイン・タブの閉鎖、再接続、文字サイズ変更、ヘルプを利用できます。

- ペインをクリックして選択し、分割バーをドラッグしてサイズを変更します。
- ドラッグで文字を選択します。TUIがマウスを利用している場合は Shift+ドラッグで選択します。
- ホイール・トラックパッドで履歴をスクロールします。マウス入力を要求するTUIにはホイール入力を送ります。
- `≡` メニューの `Smaller text` / `Larger text` で文字サイズを変更します。
- ウィンドウ・ペイン・タブを閉じても、シェルと実行中のコマンドはデーモン内に残ります。シェル自体を終了するには `exit` などを使います。

## セッションへの再接続

インストールすると、次のコマンドを利用できます。

```sh
nix profile install .
nekomata sessions list
nekomata sessions attach
nekomata sessions attach SESSION_ID
nekomata sessions detach
nekomata sessions detach SESSION_ID
```

`attach` はIDを省略するとセッション選択画面を開きます。`≡` メニューの `Attach…` からも選択できます。`detach` はIDを省略すると、コマンドを実行した端末のセッション（`FELIS_SESSION_ID`）を対象にします。対象セッションの全ビューを切断し、nekomata では対応するペインを閉じます。シェルは動いたままなので、後から再接続できます。

インストールせずに使う場合:

```sh
nix run . -- sessions list
nix run . -- sessions attach
nix run . -- sessions attach SESSION_ID
nix run . -- sessions detach SESSION_ID
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

テキスト、256色 / RGB、CJK文字、選択・コピー・貼り付け、スクロールバック、名前付きキー、マウス入力、IME確定文字の送信に対応しています。NixではUDEV Gothic NFを優先して読み込みます。macOSではフォントが見つからない場合にシステムの日本語フォントを使います。別のフォントを使う場合は `NEKOMATA_FONT=/path/to/font.ttf` を指定できます。Linuxでは Noto CJK の標準パスも探します。

このGUIは独自のテキスト描画を使います。felis本体の全描画機能を引き継ぐものではありません。Kitty画像、OSC 66文字サイズ、リガチャ、動的パレット変更、全種類の文字装飾、キー解放イベント、OSC 52クリップボード操作、SSH接続・外部からのセッション切替指示には未対応です。デーモン切断後の自動再接続と、タブ・分割配置の保存も未実装です。

IMEやOSクリップボードの操作、TUIのマウス操作、GUIの見た目は実機での追加確認が必要です。

## ライセンス

Apache-2.0。端末エンジンには [felis](https://git.natsukium.com/felis-terminal/felis)（Apache-2.0）を利用しています。上流を変更せず、固定したGit依存として参照しています。
