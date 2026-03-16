# workspace_fs

ローカルディレクトリを repository として扱う、Rust 製の安全境界つき file server。
HTTP リクエスト経由でディレクトリの編集を行う。

## 概要
- 起動時引数で repository root となる path を受け取る。
- repository root 外は触らない（ validation や sanitize を行う）。
- `REPOSITORY/.repo/` 以下は専用のディレクトリとして、 URL の指定は不可とする。
- `REPOSITORY/.repo/config.toml` で設定を書く。

> [!warning]
> この server ではユーザーの認証は行わない。
> 必要があれば認証用の wrapper を使うこと。

## API
- `/PATH/` はディレクトリに対応し、`/FILE` はファイルに対応する。柔軟な対応はしない。
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
- いずれにせよ、その途中のパスが存在しない場合はエラーとする。

## config
- `[serve]` 内で `port = 3000` のように指定する。未指定なら `3000`

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
- すべてのリクエスト:
  - 必要なら wrapper/proxy が user identity ヘッダを付ける

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
