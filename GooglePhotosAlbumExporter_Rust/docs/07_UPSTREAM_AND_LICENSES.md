# 依存関係とライセンス

| 直接依存 | 用途 |
| --- | --- |
| serde / serde_json | 設定、CDP、ページ、チェックポイント、出力 JSON の型と検証 |
| tungstenite 0.24 | ローカル CDP WebSocket。handshake 機能を使用 |
| fs2 0.4 | 出力先と専用プロファイルの実行ロック |
| tempfile 3.12.0 | 同一フォルダで一時ファイルを作成して確定する |
| sha2 0.10 / rand 0.8 | SHA-256、出力ファイル名の乱数 |

具体的な解決バージョンは Cargo.lock を参照してください。Rust 1.72.0（MSVC）で検証します。Node.js は JavaScript アダプターのテスト用です。

プロジェクト本体は MIT です。読み取り RPC のリクエスト・フィールドの対応を Google Photos Toolkit v3.3.0 の公開実装から移植し、[Toolkit の MIT ライセンス](TOOLKIT_LICENSE.txt)を同梱します。Album Backup は入出力の調査に使用しました。Chrome / Edge と Google フォトは外部製品です。

参照 [Toolkit](https://github.com/xob0t/Google-Photos-Toolkit) · [Album Backup](https://github.com/lokster/Google-Photos-Album-Backup) · [Rust の依存ロックの扱い](https://doc.rust-lang.org/cargo/guide/cargo-toml-vs-cargo-lock.html)
