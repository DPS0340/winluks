use winluks::{Error, adapter::check_consumer, image::AccessMode, probe::Filesystem};

#[test]
fn ext4_g0_failure_blocks_publication_before_driver_or_console_access() {
    for mode in [AccessMode::ReadOnly, AccessMode::ReadWrite] {
        assert_eq!(
            check_consumer(Filesystem::Ext4, mode),
            Err(Error::FsGateUnpassed)
        );
    }
}
