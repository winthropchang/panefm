use super::*;
use tempfile::tempdir;

#[test]
fn cow_error_detector_identifies_unsupported_and_cross_device() {
    let unsupported_err = io::Error::new(io::ErrorKind::Unsupported, "not supported");
    assert!(is_cow_unsupported_error(&unsupported_err));

    #[cfg(unix)]
    {
        let exdev_err = io::Error::from_raw_os_error(18);
        assert!(is_cow_unsupported_error(&exdev_err));

        let enotsup_err = io::Error::from_raw_os_error(45);
        assert!(is_cow_unsupported_error(&enotsup_err));
    }

    let not_found = io::Error::new(io::ErrorKind::NotFound, "file not found");
    assert!(!is_cow_unsupported_error(&not_found));
}

#[test]
fn cow_file_and_dir_clone_work_or_fallback_gracefully() {
    let dir = tempdir().expect("tempdir");
    let src_file = dir.path().join("source.txt");
    let dst_file = dir.path().join("target.txt");
    fs::write(&src_file, b"Hello Copy-on-Write!").expect("write src");

    let clone_result = clone_file_cow(&src_file, &dst_file);
    match clone_result {
        Ok(()) => {
            assert!(dst_file.exists());
            assert_eq!(
                fs::read_to_string(&dst_file).expect("read dst"),
                "Hello Copy-on-Write!"
            );
        }
        Err(error) => {
            assert!(
                is_cow_unsupported_error(&error),
                "若 CoW 失敗必須是可降級錯誤: {error:?}"
            );
        }
    }

    let src_dir = dir.path().join("src_folder");
    let dst_dir = dir.path().join("dst_folder");
    fs::create_dir_all(&src_dir).expect("create src dir");
    fs::write(src_dir.join("subfile.txt"), b"subcontent").expect("write sub");

    let clone_dir_result = clone_dir_cow(&src_dir, &dst_dir);
    match clone_dir_result {
        Ok(()) => {
            assert!(dst_dir.is_dir());
            assert!(dst_dir.join("subfile.txt").exists());
        }
        Err(error) => {
            assert!(
                is_cow_unsupported_error(&error),
                "若 CoW 目錄克隆失敗必須是可降級錯誤: {error:?}"
            );
        }
    }
}
