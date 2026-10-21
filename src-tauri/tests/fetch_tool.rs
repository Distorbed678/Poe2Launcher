//! Downloads the pinned DepotDownloader (about 33 MB) into a temporary directory, checks it is
//! reused the second time and that it starts (`-V`). Needs network access, so it only runs on
//! request: `cargo test --test fetch_tool -- --ignored`. It never logs in to Steam.

use std::sync::atomic::{AtomicU64, Ordering};

use poe2_launcher_lib::depot::{ToolPin, fetch_tool};

#[tokio::test]
#[ignore = "downloads ~33 MB from GitHub"]
async fn fetches_verifies_and_starts_the_pinned_tool() {
    let tmp = tempfile::tempdir().unwrap();
    let pin = ToolPin::embedded().unwrap();
    let last = AtomicU64::new(0);
    let exe = fetch_tool(&pin, tmp.path(), &|done, _| {
        last.store(done, Ordering::Relaxed)
    })
    .await
    .unwrap();
    assert!(exe.is_file());
    let asset = pin.host_asset().unwrap();
    assert_eq!(
        last.load(Ordering::Relaxed),
        asset.size,
        "the whole file was downloaded and counted"
    );

    // Second call: reused, no download.
    let calls = AtomicU64::new(0);
    let again = fetch_tool(&pin, tmp.path(), &|_, _| {
        calls.fetch_add(1, Ordering::Relaxed);
    })
    .await
    .unwrap();
    assert_eq!(again, exe);
    assert_eq!(calls.load(Ordering::Relaxed), 0);

    let out = tokio::process::Command::new(&exe)
        .arg("-V")
        .env("DOTNET_SYSTEM_GLOBALIZATION_INVARIANT", "1")
        .output()
        .await
        .unwrap();
    let text = String::from_utf8_lossy(&out.stdout);
    println!("{text}");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(text.contains("DepotDownloader v3.4.0"), "{text}");
}

/// A corrupted pin must be refused and leave nothing behind.
#[tokio::test]
#[ignore = "downloads ~33 MB from GitHub"]
async fn a_checksum_mismatch_is_refused() {
    let tmp = tempfile::tempdir().unwrap();
    let mut pin = ToolPin::embedded().unwrap();
    for a in &mut pin.assets {
        a.sha256 = "00".repeat(32);
    }
    let err = fetch_tool(&pin, tmp.path(), &|_, _| {})
        .await
        .unwrap_err()
        .to_string();
    assert!(err.contains("does not match the pinned checksum"), "{err}");
    assert_eq!(
        std::fs::read_dir(tmp.path()).unwrap().count(),
        0,
        "nothing left behind"
    );
}
