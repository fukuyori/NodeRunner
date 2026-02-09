# 🖧 Node Runner: Mainnet Protocol

Rustで作成したターミナルベースのアクションパズルゲーム。  
仕様書に基づくアーキテクチャ駆動設計。Windows / macOS / Linux 対応。

## ビルド & 実行

```bash
cd noderunner
cargo run --release
```

Linux でゲームパッドを使う場合は `libudev-dev` が必要です:
```bash
# Ubuntu / Debian
sudo apt install libudev-dev

# Fedora
sudo dnf install systemd-devel
```

ゲームパッドなしでビルド（`gilrs` を除外）:
```bash
cargo run --release --no-default-features
```

## インストール

### Linux / macOS（ローカル）
```bash
./install.sh              # ~/.local/share/noderunner/ にインストール
./install.sh --minimal    # gamepad/sound なし
./install.sh --uninstall  # アンインストール
```

### Linux パッケージ（deb / rpm）
```bash
./package.sh              # deb + rpm 両方を dist/ に生成
./package.sh deb          # deb のみ
./package.sh rpm          # rpm のみ
sudo dpkg -i dist/noderunner_0.3.2-1_amd64.deb   # Debian/Ubuntu
sudo rpm -i dist/noderunner-0.3.2-1.x86_64.rpm    # Fedora/RHEL
```

### Windows（ローカル）
```powershell
.\install.ps1             # %LOCALAPPDATA%\NodeRunner\ にインストール
.\install.ps1 -Uninstall  # アンインストール
```

### Windows（MSI インストーラ）
```powershell
.\build-msi.ps1           # dist\noderunner-0.3.2.msi を生成
msiexec /i dist\noderunner-0.3.2.msi              # インストール
msiexec /i dist\noderunner-0.3.2.msi /qn          # サイレント
```
WiX Toolset v3 または v4 が必要です。

## 操作方法

### キーボード

| キー | アクション |
|------|-----------|
| `←→↑↓` / `WASD` | 移動・ハシゴ昇降・ロープ移動 |
| `Z` / `Q` | 左下をハック |
| `X` / `E` | 右下をハック |
| `R` | レベルリスタート |
| `ESC` | メニューに戻る / 終了 |

### ファンクションキー

| キー | アクション |
|------|-----------|
| `F1` | ポーズ / 再開 |
| `F2` | レベルリスタート |
| `F3` | レベルパック選択 |
| `F4` | レベル選択画面へ |
| `F5`〜`F8` | スロット1〜4にセーブ |
| `F9`〜`F12` | スロット1〜4からロード |

ポーズ中も `F3`（パック選択）、`F5`〜`F8`（セーブ）、`F9`〜`F12`（ロード）が使えます。  
タイトル画面では `F9`〜`F12` でセーブデータをロードできます。

### ゲームパッド

Xbox / PlayStation / Switch Pro / 汎用 HID コントローラー対応（`gilrs`クレート経由）。

| 入力 | アクション |
|------|-----------|
| D-pad / 左スティック | 移動 |
| B / Y / L1 | 左をハック |
| A / X / R1 | 右をハック |
| Start | 決定・リスタート |
| Start | リスタート |
| Select | 終了 |

## ゲームルール

**ジャンプは存在しない**。これが最重要の設計制約。

- **トークン** (`$`) を全てマイニング → 脱出口（隠しハシゴ）が出現
- **画面最上部**に到達でノードクリア
- **ハック**はファイアウォールのみ有効。段階的にひび割れ→崩壊→穴が開く
- 穴は一定時間で再生する
- **センチネル** (`♂`) に接触するとミス。穴に落とすと一時的に拘束
- 穴が塞がる時に中にいるとセンチネルは消滅（しばらくしてリスポーン）
- センチネルがトークンを拾うことがある。穴に落とすとドロップ

## アーキテクチャ

```
├── Cargo.toml
├── config.toml              # 速度・ボタン設定（外部ファイル）
├── install.sh               # Linux/macOS インストーラ
├── install.ps1              # Windows インストーラ
├── package.sh               # deb/rpm パッケージビルダ
├── build-msi.ps1            # Windows MSI ビルダ
├── levels/                  # レベルファイル（外部、.txt）
│   ├── 001_level1.txt
│   ├── 002_level2.txt
│   └── ... (155 levels)
├── packs/                   # レベルパック（.nlp）
│   └── classic_challenge.nlp
└── src/
    ├── main.rs              # IOレイヤ: ゲームループ・入力マッピング
    ├── config.rs            # config.toml読み込み
    ├── domain/              # ドメイン: エンジン非依存のゲームルール
    │   ├── tile.rs          # タイル種別とプロパティクエリ
    │   ├── entity.rs        # エンティティ定義・状態マシン
    │   ├── rules.rs         # 移動ルール・ハックルール（純粋関数）
    │   └── ai.rs            # ガードAI (BFS経路探索)
    ├── sim/                 # シミュレーション: 1フレームを進める
    │   ├── world.rs         # WorldState（全状態のスナップショット）
    │   ├── step.rs          # Step関数（固定処理順序）
    │   ├── event.rs         # イベント定義
    │   ├── level.rs         # レベルローダ（外部ファイル / 内蔵フォールバック）
    │   └── save.rs          # セーブ/ロード（スロット式 + レガシー）
    └── ui/                  # プレゼンテーション: 入力・描画
        ├── input.rs         # キーボード入力状態トラッカー
        ├── gamepad.rs       # ゲームパッド入力 (gilrs, optional)
        ├── renderer.rs      # crossterm描画（ダブルバッファ・差分更新）
        └── sound.rs         # 効果音 (rodio, optional)
```

### 設計原則（仕様書準拠）

1. **グリッド世界が真実**: 当たり判定・移動・AIはタイル座標中心
2. **フレームで積分しない**: 連続物理ではなく移動ルールで決定
3. **状態を第一級に**: `OnGround` / `Falling` / `OnLadder` / `OnRope` / `InHole` / `Dead`
4. **イベント駆動**: ルールとプレゼンテーションの分離
5. **穴はエンティティ**: タイル書き換えではなくHoleエンティティで管理

### 1フレームの処理順序

```
入力収集 → 意図生成 → ハック解決 → 移動解決 → 重力解決
→ 衝突判定 → 穴効果 → タイマ更新 → 勝利判定 → イベント配信
```

## 設定ファイル（config.toml）

実行ファイルと同じディレクトリに `config.toml` を置くと、速度やボタンをカスタマイズできます。  
ファイルがなくてもデフォルト値で起動します。一部の項目だけ書くことも可能です。

```toml
[general]
levels_dir = "levels"      # レベルファイルのディレクトリ（相対 or 絶対）

[speed]
tick_rate_ms       = 75    # メインループ間隔 (ms)。小さいほど高速
player_move_rate   = 2     # プレイヤーがN tickに1回移動
guard_move_rate    = 5     # センチネルがN tickに1回移動
dig_duration       = 5     # ハック完了までのtick数
hole_regen_ticks   = 150   # 穴が塞がるまでのtick数
trap_escape_ticks  = 120   # 捕獲されたセンチネルの脱出tick数
guard_respawn_ticks = 80   # 消滅したセンチネルのリスポーンtick数

[gamepad]
# ボタン名: A, B, X, Y, L1, R1, L2, R2, Start, Select
# gilrsマッピング:
#   A=South(Xbox A/PS ×)  B=East(Xbox B/PS ○)
#   X=West(Xbox X/PS □)   Y=North(Xbox Y/PS △)
hack_left  = ["B", "Y", "L1"]
hack_right = ["A", "X", "R1"]
confirm    = ["Start"]
cancel     = ["Select"]
restart    = ["Start"]
```

## レベル追加

`levels/` ディレクトリに `.txt` ファイルを追加するだけで、新ノードが登場します。  
ファイル名のアルファベット順がステージ順になります（`01_xxx.txt`, `02_xxx.txt`, ...）。

### ファイル形式

```
# Node Name Here
         ^        ^         
                            
   $     ----------     $   
  ###    H        H    ###  
         H   $    H        
   ... (28文字幅 × 16行) ...
============================
```

- 1行目: `# ノード名`（`#` の後にスペースと名前）
- 2行目以降: マップデータ（16行、各28文字幅）

### マップ記号

| 文字 | 意味 |
|------|------|
| ` ` | 空白（通行可・落下） |
| `#` | ファイアウォール（立てる・掘れる） |
| `=` | コンクリート（掘れない） |
| `H` | ハシゴ（上下移動） |
| `-` | ロープ（横移動） |
| `$` | トークン |
| `P` | プレイヤー開始位置 |
| `E` | センチネル開始位置 |
| `^` | 脱出ハシゴ列マーカー（指定列のみ延長） |
| `T` | トラップ（見た目は`#`と同じ、上に乗ると崩落） |

`^` を置かない場合、全ハシゴ列が延長されます（フォールバック動作）。

## 拡張ポイント

仕様書に従い、オリジナルを壊さず拡張可能な領域：

- **リプレイ**: `step()`の入力を記録するだけで再現可能
- **アンドゥ**: `WorldState`をクローンしてスタック管理
- **ステージエディタ**: テキストファイルを編集するだけ
- **解法可視化**: AIのBFS結果を描画レイヤで表示
