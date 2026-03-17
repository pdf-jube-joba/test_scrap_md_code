# workspace_fs

ローカルディレクトリを repository として扱う、Rust 製の安全境界つき file server。
HTTP リクエスト経由でディレクトリの編集を行う。

## 概要
- 起動時引数で repository root となる path を受け取る。
- repository root 外は触らない（ validation や sanitize を行う）。
- `REPOSITORY/.repo/` 以下は専用のディレクトリとする。
- config で細かい指定ができる。
- plugin で hook を書いて生成などをすることができるようにする。
- task で起動前に plugin invoke ができるようにする。

> [!warning]
> このサーバーではユーザーの認証・https 化は行わない。
> 必要があれば wrapper を通すこと。

## API
全てのリクエストで `user-identity` （文字列）を設定すること。
> [!warning]
> もし設定されていないならリクエストを拒否する。

- `/PATH/` はディレクトリに対応し、`/FILE` はファイルに対応する。
- `GET URL` は内容の取得
  - `GET /dir/` なら、 ディレクトリ直下の内容を 1 entry 1 line で返す。
  - `GET /file.txt` ならファイルの内容をそのまま返す。
- `POST URL` は新規作成
  - `POST /dir/` ならディレクトリを新規作成する。
  - `POST /file.txt` ならファイルを新規作成する。
  - いずれにせよ、すでに存在していたらエラーとする。
- `PUT /file.txt` で既存ファイルを更新する。
  - 存在しない場合はエラーとする。
- `DELETE URL` は削除。
  - `DELETE /dir/` ならディレクトリを削除する、**ただし、空のディレクトリのときだけ。**
  - `DELETE /file.txt` ならファイルを削除する。

> [!warning]
> 柔軟な対応はしない。愚直に対応する。
> - ファイルでの指定でディレクトリが見つかったときは、ディレクトリに直さずにエラーにする。
> - URL の途中のパスが存在しない場合はエラーとする。

> [!warning]
> URL としては `.repo/` は一切指定できないものとする。

## config
`REPOSITORY/.repo/config.toml` で設定を書く。

### serve
port を指定する。未指定なら `3000`
```
[serve]
port = 3030
```
### policy
path に対して API 経由での GET/POST/DELETE/PUT をやってよいかを指定できる。
```
[[policy]]
path = ".git/"
GET = false
POST = false
PUT = false
DELETE = false
```
なお、 `.repo/` 以下は設定できない。
そもそも API でも `.repo/` 以下はエラーとする。

### mount
path の転送を行う。
```
[[mount]]
url_prefix = "/assets/"
source = ".repo/generated/assets/"
```
- mount 先は `.repo/generated/` 以下とし、その外は不可とする。
- URL は何でもいいが、そこに対しては、 **GET のみ**できることとする。

例えば上の例では、 `/assets/*` へのアクセスは `.repo/generated/assets/*` へのアクセスにして、 GET のみが許される。
glob は指定できない。

> [!warning]
> `url_prefix` がすでに `REPOSITORY/` に存在するディレクトリとかぶったらエラーとする。
> これは serve 前に検知してエラーを吐いて終了すること。

### plugin
hook のような形で、プラグインを記述する。内部では `{PLACE_HOLDER}` の記法が使える。
```
[[plugin]]
name = "convert-md-html"
runner = "command"
command = ["python3", "./convert-md-html.py", "{GET.PATH}"]
trigger = "GET"
path = "*.md"
```
上のプラグインは外部コマンドの invoke を行う：
`*.md` に該当する GET があったときに `{GET.PATH}` をファイル名で置き換えて実行する。

> [!note]
> 将来的には、 `"command"` じゃなくて wasm も指定できるとうれしいが、 interface を考えるのが難しい。

### task
plugin をどの順番に実行するかを書いて、起動時に指定する。
```
[[task]]
name = "build"
steps = ["build-wasm", "build-autosummary"]
```

## plugin
`config.toml` で指定したもののみを対象とする。

実行するタイミングは、
1. `trigger = "manual"` 以外の場合は、特定の API 操作が呼ばれたとき。
2. `trigger = "manual"` の場合には、
  - `POST /.plugin/<PLUGIN_NAME>/run` が来た時
  - `task` で指定されたとき... serve の前に行われる。

> [!warning]
> plugin が書き換えるのは
> - `.repo/generated/<PLUGIN_NAME>` ... mount で使える、 API で露出するようのディレクトリ：最終成果物など
> - `.repo/cache/<PLUGIN_NAME>` ... API 経由では触れないディレクトリ：中間成果物やキャッシュなど
> それ以外の書き換えは自己責任とする。

例：
```
[[plugin]]
name = "wasm-build"
runner = "command"
command = ["cargo", "build", "--target", "wasm32-unknown-unknown"]
trigger = "manual"
```
これは明らかに `REPOSITORY_ROOT/target/` を書き換えるが、無視する。
同様に、 git を使って自動で履歴保存とかも同じようになるはず。

### place holder について
基本的には trigger ごと設定できる項目を分けて、ここに乗っているもの以外は評価をしない。

全体で使えるもの
- `REPOSITORY_NAME`
- `PLUGIN_NAME`
- `OUTPOST_DIRECTORY` ... `.repo/generated/<PLUGIN_NAME>` のこと

GET
- `GET.PATH`
- `GET.USER-IDENTITY`

POST/PUT/DELETE も同様のものだけ実装する。

## task
task が指定されたときに、 serve 前に指定された plugin を順番に起動する。

## 起動方法

```bash
cargo run -- ./test-repository
```

起動後の例:

- `GET /`:
  - repository root 直下の一覧
- `GET /docs/`:
  - `docs` 直下の一覧
- `POST /notes/`:
  - `notes` ディレクトリを作成
- `GET /index.md`:
  - `index.md` の本文
- `PUT /index.md`:
  - 既存の `index.md` を上書き保存
- `POST /new.md`:
  - `new.md` を新規作成
- `DELETE /notes/`:
  - 空の `notes` ディレクトリを削除
- `DELETE /new.md`:
  - `new.md` を削除

# 実装について

## Rust の責務分割

- HTTP 層はルーティングとプレーンテキスト入出力だけを担当する
- repository のパス解決、一覧、読込、作成、更新、削除は `Repository` trait の実装に閉じ込める
- `config.toml` の読込は `config` module の専用構造体で扱う
- wrapper/proxy から渡された user identity の取込みは `identity` module で扱う
- 現在はファイルシステム実装として `FsRepository` を使う
- 将来的に別実装を足しても、HTTP 層は trait 越しに扱う

## Identity

- この server 自体は認証しない
- 外部 wrapper/proxy が認証済みユーザーをヘッダで渡す前提にする
- 現在は request ごとに user identity を `String` として request context に積むだけにする
- user identity のヘッダ名は `user-identity` に固定する

例:

```http
user-identity: alice
```

## パス安全性

- `..` を含む path は拒否する
- 絶対パスは拒否する
- `.repo/` 配下は API から直接触れない
- 保存時も repository 相対パスを正規化してから処理する

## 今後の拡張

- plugin / hook system
- 履歴管理 plugin
- git backend plugin
- wasm component による安全な拡張実行
