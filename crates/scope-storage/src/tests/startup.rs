use super::*;
use crate::lifecycle::open_local_root_with_cleanup;

#[tokio::test]
async fn startup_cleanup_does_not_race_an_open_store_ingesting_a_pack() {
    let fixture = Fixture::new(8, 64, 1);
    let root = fixture.local_root.clone();
    tokio::fs::create_dir_all(root.join("staging"))
        .await
        .unwrap();
    let (unlinked, emptied) = tokio::sync::oneshot::channel();
    let (resume, resumed) = std::sync::mpsc::channel();
    let cleanup = tokio::task::spawn_blocking(move || {
        let mut unlinked = Some(unlinked);
        open_local_root_with_cleanup(&root, |path| {
            if path == root.join("staging") {
                for entry in std::fs::read_dir(path)? {
                    let entry = entry?;
                    if entry.file_type()?.is_dir() {
                        std::fs::remove_dir_all(entry.path())?;
                    } else {
                        std::fs::remove_file(entry.path())?;
                    }
                }
                let _ = unlinked.take().unwrap().send(());
                resumed.recv().unwrap();
                std::fs::remove_dir(path)
            } else {
                std::fs::remove_dir_all(path)
            }
        })
    });
    let _ = emptied.await;
    let input = b"API pack created while the worker starts";
    let staged = fixture
        .store
        .ingest(REPOSITORY_ID, &input[..], u64::MAX)
        .await;
    let _ = resume.send(());
    cleanup.await.unwrap().unwrap();
    assert_eq!(
        tokio::fs::read(staged.unwrap().local_pack_path())
            .await
            .unwrap(),
        input
    );
}
