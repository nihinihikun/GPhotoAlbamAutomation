# API 調査と実装根拠

| 候補 | 今回の要件への適用 |
| --- | --- |
| Photos Library API | 2025年3月31日以降の一覧・参照はアプリ作成済みのアルバムとメディアに限定されます。既存ライブラリを列挙できません。[公式の変更履歴](https://developers.google.com/photos/support/updates) |
| Photos Picker API | 利用者が Picker セッションで選んだメディアを一覧化する仕組みです。既存アルバムの全所属関係を無人で返すメソッドは公開仕様にありません。[公式の導入手順](https://developers.google.com/photos/picker/guides/get-started-picker) |
| Data Portability API | OAuth 同意後にサービス単位でアーカイブを要求する API です。公開スコープ・スキーマ一覧に Google フォトのアルバム構成を取得する対象を確認できませんでした。[公開スコープ](https://developers.google.com/data-portability/user-guide/scopes) · [スキーマ一覧](https://developers.google.com/data-portability/schema-reference) |
| Google フォト Web の非公開 RPC | 公開 Toolkit 実装で、既存アルバム一覧、所属ページ、バッチと単件のメディア詳細の読み取り RPC を確認しました。認証済みの専用ブラウザで、この4種類を利用します。[Toolkit API v3.3.0](https://github.com/xob0t/Google-Photos-Toolkit/blob/v3.3.0/src/api/api.ts) |

## 非公開 RPC の対応

src/photos.js にルート、リクエスト構造、レスポンスの射影があります。送信先は PhotosUi の batchexecute で、同一オリジン、アカウントスロット、およびページから取得したルートを照合します。f.req に RPC ID と JSON 配列をフォーム符号化します。ページのセッション値はリクエストのみに使います。受信側は XSSI プレフィックスを確認し、対象 RPC と一致する wrb.fr レコードの JSON を1つだけ受け入れます。

| RPC | リクエストとレスポンスの参照箇所 |
| --- | --- |
| Z5xsfc | Toolkit getAlbums と同じページサイズ100、継続トークン、固定フィルター。応答 [0] がアルバム一覧、[1] が継続。行の [0] が ID、末尾の 72930366 にタイトル、申告数、共有フラグ、閲覧キーがあります。総アルバム数の確かなフィールドは確認できていません。 |
| snAcKc | Toolkit getAlbumPage と同じアルバム ID、継続、共有閲覧キー。応答 [1] がメンバー、[2] が継続、[3] にアルバム ID と申告数。先頭ページの ID を照合し、明示的な0件のアルバムを許可します。 |
| EWgK9e | Toolkit getBatchMediaInfo の ID リストとフィールド指定。各行の ID とファイル名、撮影・作成時刻、サイズを読むだけです。40件以内に分割します。 |
| fDcn4b | Toolkit getItemInfoExt と同じメディア ID と閲覧キーを使い、バッチでファイル名が取れなかった項目の詳細を1件ずつ照会します。応答 [0] の行から ID、ファイル名、撮影時刻、サイズを取得します。 |

マッピングの参照 [Toolkit の解析コード](https://github.com/xob0t/Google-Photos-Toolkit/blob/6db9ab3/src/api/parser.ts) · [ページ変数](https://github.com/xob0t/Google-Photos-Toolkit/blob/v3.3.0/src/windowGlobalData.ts)。Google の安定仕様ではありません。型、エンベロープ、応答 ID、件数や継続の不一致はレポートにエラーを記録します。共有は一覧 RPC が返す範囲です。共有リンク専用の一覧や、列挙対象外のアルバムは対象範囲を別途確認する必要があります。

実アカウントの小規模照会では、ID が null の詳細行やデータのない詳細応答がありました。その場合はファイル名などを取得不能として扱います。照会順による割り当ては行わず、応答 ID が一致した項目だけを使います。
