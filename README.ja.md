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

有効化後、zshで通常通り入力したコマンドは透過的に `reout capture -- <command>` 経由で実行されます。`reout` 自身のコマンドはキャプチャ対象から除外されます。

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

削除:

```bash
reout delete 42
reout clear
```

通常のUnix CLIとしてパイプできます。

```bash
reout --plain | grep ERROR
reout show 42 | rg timeout
```

## 設計メモ

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

### 2. PTY Proxy

stdinがターミナルの場合、`reout capture` は疑似ターミナルを開き、`$SHELL -lc <command>` でコマンドを実行します。PTYから読んだ出力を実ターミナルへ転送しつつ、同じバイト列をSQLiteへ保存します。

単純なstdout redirectよりも、TTY前提のCLIの挙動を保ちやすい構成です。`SIGWINCH` によるterminal resizeを転送し、stdinはraw modeにしてCtrl+CやCtrl+ZをPTY側へ通します。

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

長時間動く対話セッションは大きな履歴レコードを作る可能性があります。retentionとignore設定は今後実装予定です。

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

将来の設定案:

```toml
[ignore]
commands = [
  "reout*",
  "cat .env*",
  "aws configure*"
]

[retention]
max_age = "30d"
max_bytes = "1GB"
```

## 既知の制約

- 対話中のzshに定義されたaliasやfunctionは、常に直接実行時と完全に同じ挙動になるとは限りません。キャプチャ対象コマンドは `$SHELL -lc` 経由で実行されます。
- MVPでは `stdout` と `stderr` を別々には保存しません。
- クリップボードコピーは現時点ではmacOSの `pbcopy` を使います。
- retention policyは未実装です。大きなコマンド出力は手動削除するまで残ります。

## 将来のコマンド

現在のschemaとコマンドモデルは、次のような拡張を想定しています。

```bash
reout --failed
reout --cwd .
reout --since 1d
reout --last cargo
reout --last terraform
reout --json
reout --md
reout --errors
reout prune
```

`--last terraform` はTerraform専用パーサではなく、任意コマンドに使えるcommand prefixまたはsubstring検索として扱う方針です。
