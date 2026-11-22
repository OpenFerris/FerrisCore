use super::OwnerRegistry;

#[test]
fn write_read_snapshot_and_flags_track_the_entries() {
    let reg: OwnerRegistry<u32> = OwnerRegistry::new();
    assert!(reg.is_empty());
    assert_eq!(reg.len(), 0);
    assert!(!reg.is_registered());

    reg.write(|entries| {
        entries.push(1);
        entries.push(2);
    });
    reg.mark_registered();

    assert!(!reg.is_empty());
    assert_eq!(reg.len(), 2);
    assert!(reg.is_registered());
    assert_eq!(reg.snapshot(), vec![1, 2]);
    assert_eq!(reg.read(|entries| entries[0]), 1);

    reg.reset();
    assert!(reg.is_empty());
    assert_eq!(reg.len(), 0);
    assert!(!reg.is_registered());
}

#[test]
fn a_poisoned_lock_is_recovered_with_into_inner() {
    let reg: OwnerRegistry<u32> = OwnerRegistry::new();
    let panicked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        reg.write(|entries| {
            entries.push(7);
            panic!("poison the registry lock on purpose");
        });
    }));
    assert!(panicked.is_err(), "the closure must have panicked");

    // The lock is poisoned now; every accessor must recover it instead of
    // re-panicking, so the entries committed before the poison survive.
    assert_eq!(reg.len(), 1);
    assert_eq!(reg.snapshot(), vec![7]);
    assert!(!reg.is_empty());
}
