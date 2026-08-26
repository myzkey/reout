# reout

[English](README.md)

Command output should be reusable after execution.

`reout` は、ターミナルで実行したコマンドの出力をローカルに保存し、コマンド実行後に再表示・コピー・一覧表示・検索できるようにするCLIです。

コアは特定コマンドに依存しません。Terraform、Git、Docker、Kubernetes、テストランナー、コンパイラ、パッケージマネージャ、独自CLIなどを、すべて次の汎用データとして扱います。

- command
- cwd
- terminal output
- exit code
- started_at
- finished_at

## MVP

現在の対象環境:

- macOS
- zsh
- SQLite保存先は `$XDG_DATA_HOME/reout/reout.db` または `~/.local/share/reout/reout.db`
- 完全ローカル保存
- stdinがターミナルの場合はPTY経由で実行

ローカルインストール:

```bash
cargo install --path .
```

zsh integrationを有効化:

```bash
eval "$(reout init zsh)"
```

有効化後、zshで通常通り入力したコマンドは透過的に `reout` 経由へ振り分けられます。`reout` 自身のコマンドはキャプチャ対象から除外されます。

zsh integrationには2つのキャプチャ経路があります。

- 外部コマンドはPTYを使う `reout capture -- <command>` 経由で実行します。
- 現在のshellに定義されたalias/functionは現在のzshプロセス内で実行し、`reout import` で保存します。

一時的に無効化:

```bash
REOUT_DISABLED=1
```

## 使い方

直前の出力を表示:

```bash
reout
```

直前の出力をコピー:

```bash
reout -c
reout --copy
```

ANSIエスケープを除去:

```bash
reout --plain
reout --plain -c
```

少し前の出力を表示:

```bash
reout -2
reout --offset 2
```

履歴を表示:

```bash
reout list
```

ID指定で表示・コピー:

```bash
reout show 42
reout copy 42
```

command文字列で検索:

```bash
reout find test
reout find kubectl
reout find "terraform plan"
```

履歴を絞り込み:

```bash
reout --failed
reout --cwd .
reout --since 1d
reout --last cargo
reout list --failed --since 1w
```

構造化出力:

```bash
reout --json
reout --md
reout list --json
reout show 42 --md
```

簡易error行抽出:

```bash
reout --errors
reout --errors -c
```

削除:

```bash
reout delete 42
reout clear
reout prune --older-than 30d
```

retentionを設定している場合、capture/importのたびに自動pruneも実行されます。

通常のUnix CLIとしてパイプできます。

```bash
reout --plain | grep ERROR
reout show 42 | rg timeout
```

## 設計メモ

### アーキテクチャ

実装はクリーンアーキテクチャ寄りに分割しています。

```text
src/
  domain/          captured command outputなどの中核エンティティ
  application/     ユースケースとport。SQLite、PTY、clapには依存しない
  infrastructure/  SQLite、PTY command runner、clipboard、paths、ANSI stripping
  presentation/    clap CLI、shell init script、terminal table formatting
  main.rs          薄いprocess entrypoint
```

依存方向:

```text
presentation -> application -> domain
infrastructure -> application -> domain
```

application層は `CommandRepository`、`CommandRunner`、`Clipboard`、`OutputSanitizer` といったtraitに依存します。SQLite、PTY、`pbcopy` はそれらのportを実装するadapterです。

### 1. Shell Integration

zshのZLE widgetを使うと、シェルが実行する前の入力行を置き換えられます。MVPでは独自の `accept-line` widgetを入れ、次のような入力:

```bash
any-command
```

を次の形に置き換えます。

```bash
reout capture -- any-command
```

これにより、ユーザーが毎回 `reout run ...` のようなラッパーコマンドを手入力する必要を避けています。

integrationは `cd`、`export`、`alias`、`source`、`jobs`、`fg`、background jobなどのshell状態に関わるコマンドをスキップします。これらはcapture proxyではなく、現在のshellで実行される必要があります。また、通常のターミナル利用を壊さないため、`vim`、`less`、`top`、`ssh`、`fzf`、`tmux`、`screen` などのfull-screenまたはsession-orientedな対話コマンドもスキップします。

### 2. PTY Proxy

stdinがターミナルの場合、`reout capture` は疑似ターミナルを開き、`$SHELL -lc <command>` でコマンドを実行します。PTYから読んだ出力を実ターミナルへ転送しつつ、同じバイト列をSQLiteへ保存します。

単純なstdout redirectよりも、TTY前提のCLIの挙動を保ちやすい構成です。`SIGWINCH` によるterminal resizeを転送し、stdinはraw modeにします。Ctrl+Cは転送され、exit code `130` として保存されます。Ctrl+Zは `reout capture` 配下でproxyが停止状態のまま固まらないよう、Ctrl+C相当に変換します。

### 3. stdout / stderr の順序

MVPではPTY masterから読んだ1本の `output` BLOBを保存します。stdout/stderrを別々のpipeで読むより、ユーザーが実際に見た順序に近い出力を保存しやすいためです。

非TTY実行では通常プロセス実行へフォールバックし、stdout/stderrを結合して保存します。これは自動検証やスクリプト用途を扱いやすくするためで、主経路は対話ターミナルでのPTY実行です。

### 4. Interactive Commands

PTY経路は次のようなコマンドで通常のターミナル体験を壊しにくくすることを意図しています。

```bash
vim
less
top
ssh
fzf
git add -p
docker compose logs -f
kubectl logs -f
```

長時間動く対話セッションは大きな履歴レコードを作る可能性があります。保存したくないコマンドにはignore ruleとretention pruningを使います。

### 5. SQLite Schema

```sql
CREATE TABLE commands (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  command TEXT NOT NULL,
  cwd TEXT NOT NULL,
  output BLOB NOT NULL,
  exit_code INTEGER NOT NULL,
  started_at INTEGER NOT NULL,
  finished_at INTEGER NOT NULL
);
```

### 6. Atuinなどの関連ツール

Atuinはshell lifecycle hookを使ってコマンド履歴のメタデータを記録します。`reout` もshell integrationという考え方は近いですが、主対象はターミナル出力そのものです。

preexec/precmd hookだけではコマンド実行後の出力を取得できないため、MVPではZLEによるコマンドライン置き換えを使っています。

### 7. Security

`reout` はキャプチャした出力を外部へ送信しません。`reout` が所有するデータベースディレクトリはprivate permissionで作成し、Unix環境ではDBファイルを `0600` にします。

設定形式:

```toml
[ignore]
commands = [
  "reout*",
  "cat .env*",
  "aws configure*"
]

[retention]
max_age = "30d"
max_entries = 1000
max_bytes = "1gb"

[capture]
max_output_bytes = "10mb"
```

## 既知の制約

- 現在のshellに定義されたalias/functionは非PTYのimport経路で対応します。この経路ではalias/function解決を維持できますが、stdoutはTTYとして見えません。
- MVPでは `stdout` と `stderr` を別々には保存しません。
- クリップボードコピーは現時点ではmacOSの `pbcopy` を使います。
- 透明なshell integrationはzshのみです。`reout init bash` と `reout init fish` は未実装であることを明示します。
- `--errors` は保存済みoutputに対する単純な行filterです。stderr抽出ではありません。

## ローカル検証済み

- `cargo check`
- `cargo test`
- `cargo clippy -- -D warnings`
- 非TTYでのcapture、直前出力、offset出力、list、find、plain output
- 対話zsh integrationでの `cd`、`echo`、`reout --plain`、`reout list`
- 対話zsh integrationでの現在shellのalias/function
- PTY stdin forwardingを `cat` で確認
- Ctrl+C forwardingを `sleep 10` で確認
- Ctrl+Z hang avoidanceを `sleep 10` で確認
- `REOUT_CONFIG` によるconfig ignore pattern
- JSON、Markdown、failed、cwd、since、last、errors、prune commands

## Config

default config path:

```bash
reout config-path
```

設定例:

```toml
[ignore]
commands = [
  "reout*",
  "cat .env*",
  "aws configure*"
]

[retention]
max_age = "30d"
```

retentionを手動実行:

```bash
reout prune
```

設定済みretentionは `reout capture` と `reout import` の後にも自動適用されます。

`--last terraform` はTerraform専用パーサではなく、任意コマンドに使えるcommand substring検索です。
