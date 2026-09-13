# screen brightness control

[English](README.en.md)

Rust で書かれた Windows 向けモニター輝度制御ライブラリです。

モニターと接続が DDC/CI に対応している場合は DDC/CI を使用します。DDC/CI が
利用できない場合は、バックライトの電力を変えずに画面のまぶしさを抑える
ソフトウェアオーバーレイにフォールバックします。

オーバーレイによる減光はホストプロセスが動作している間だけ有効です。プロセス
が終了すると、オーバーレイウィンドウは自動的に削除されます。

## ワークスペース構成

```
crates/
  brightness-core/   ライブラリ
  brightness-cli/    デバッグ用コンソール
```

## ビルド

```bash
cargo build
```

## CLI

```bash
cargo run -p brightness-cli -- list
cargo run -p brightness-cli -- get 0
cargo run -p brightness-cli -- set 0 50
cargo run -p brightness-cli -- watch
```

`watch` コマンドはモニターの接続と切断を監視し、モニター一覧を自動的に
更新します。

## ライブラリの使い方

アプリケーションに `brightness-core` を追加します:

```toml
[dependencies]
brightness-core = { path = "../crates/brightness-core" }
```

基本的な例:

```rust
use brightness_core::MonitorManager;

fn main() -> brightness_core::Result<()> {
    let mut manager = MonitorManager::new()?;

    for monitor in manager.list_monitors()? {
        println!("{} [{}]", monitor.name, monitor.id);
    }

    manager.set_brightness("0", 80)?;
    Ok(())
}
```

### モニター ID

各モニターには `ddc:display1` や `overlay:display1` のような安定した文字列
ID があります。ほとんどの API では、一覧のインデックスや名前の一部でも
指定できます。

### スライダーのドラッグ

ドラッグ中は非同期書き込み、離したときに同期書き込みを使います:

```rust
// ドラッグ中
manager.set_brightness_async(&monitor_id, value)?;

// 離したとき
manager.set_brightness(&monitor_id, value)?;
```

### ホットプラグ

UI タイマーやバックグラウンドスレッドから watcher をポーリングします:

```rust
let watcher = manager.watch_hotplug()?;

if watcher.try_recv().is_some() {
    manager.refresh()?;
    // ここでコンボボックスとスライダーを再構築する
}
```

`refresh` はキャッシュされた DDC ハンドルを解放し、切断されたディスプレイの
オーバーレイウィンドウを削除し、残っているオーバーレイ対象に減光を
再適用します。

### オーバーレイを使うアプリケーション

オーバーレイ対象があり、ユーザーが 100% 未満の輝度を設定した場合は、プロセス
を存続させてください (トレイアプリなど)。DDC 対象ではこの対応は不要です。

## API 一覧

| メソッド | 説明 |
|--------|------|
| `new` | マネージャーを作成しモニターをスキャン |
| `list_monitors` | モニターと制御方式の一覧 |
| `get_brightness` | 輝度 0-100 を読み取り |
| `set_brightness` | 輝度を設定し DDC の反映を待つ |
| `set_brightness_async` | スライダードラッグ用に輝度変更をキュー |
| `adjust_brightness` | 差分で輝度を変更 |
| `refresh` | ホットプラグやレイアウト変更後に再スキャン |
| `watch_hotplug` | 接続と切断の通知を受け取る |
| `resolve_id` | インデックスや名前をモニター ID に解決 |

## プラットフォーム対応

Windows のみ。それ以外のプラットフォームでは `Error::PlatformUnsupported` を
返します。

## DDC/CI に関する注意

DDC/CI はモニター、ケーブル、ドライバーに依存します。DisplayPort と DVI は
通常 HDMI より動作しやすいです。一部のモニターでは OSD メニューで DDC/CI を
有効にする必要があります。
