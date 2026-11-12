#[path = "../shared/pci_readiness.rs"]
mod readiness;
use readiness::*;
fn header(id: u16, next: usize) -> u32 {
    id as u32 | (1 << 16) | (next as u32) << 20
}
#[test]
fn absent_and_enabled_extensions_are_distinct_observations() {
    assert_eq!(extensions(|_| Ok(0)), Ok(Extensions::default()));
    let mut config = [0u32; 1024];
    config[0x100 / 4] = header(0x0f, 0x108);
    config[0x104 / 4] = 1 << 31;
    config[0x108 / 4] = header(0x13, 0x118);
    config[0x10c / 4] = 1;
    config[0x118 / 4] = header(0x1b, 0);
    config[0x11c / 4] = 1 << 16;
    let observed = extensions(|at| Ok(config[at / 4])).unwrap();
    assert_eq!(
        (observed.ats, observed.pri, observed.pasid),
        (Some(true), Some(true), Some(true))
    );
    config[0x104 / 4] = 0;
    config[0x10c / 4] = 0;
    config[0x11c / 4] = 0;
    let observed = extensions(|at| Ok(config[at / 4])).unwrap();
    assert_eq!(
        (observed.ats, observed.pri, observed.pasid),
        (Some(false), Some(false), Some(false))
    );
}
#[test]
fn acs_reports_capability_and_control_without_claiming_isolation() {
    let observed = extensions(|at| {
        Ok(if at == 0x100 {
            header(0x0d, 0)
        } else {
            0x000d_007f
        })
    })
    .unwrap();
    assert_eq!(
        observed.acs,
        Some(Acs {
            capability: 0x7f,
            control: 0x0d
        })
    );
}
#[test]
fn malformed_cycles_overlap_versions_and_reads_reject() {
    assert_eq!(extensions(|_| Err(Error::Read)), Err(Error::Read));
    assert_eq!(extensions(|_| Ok(header(1, 0x100))), Err(Error::Chain));
    assert_eq!(extensions(|_| Ok(0x0f)), Err(Error::Version(0x0f)));
    assert_eq!(
        extensions(|_| Ok(0x0f | (2 << 16))),
        Err(Error::Version(0x0f))
    );
    assert_eq!(
        extensions(|at| Ok(if at == 0x100 {
            header(0xf, 0x104)
        } else {
            header(1, 0)
        })),
        Err(Error::Chain)
    );
    assert_eq!(
        extensions(|at| Ok(if at == 0x100 {
            header(0xf, 0x108)
        } else if at == 0x108 {
            header(0xf, 0)
        } else {
            0
        })),
        Err(Error::Duplicate(0xf))
    );
    assert_eq!(
        extensions(|at| Ok(if at == 0x100 {
            header(1, 0xffc)
        } else {
            header(0xf, 0)
        })),
        Err(Error::Chain)
    );
}
fn bridge(rid: u16, secondary: u8, subordinate: u8) -> Function {
    Function {
        segment: 0,
        requester: rid,
        header: 1,
        primary: (rid >> 8) as u8,
        secondary,
        subordinate,
    }
}
#[test]
fn root_and_nested_paths_are_bounded_and_ordered() {
    let functions = [bridge(0x08, 1, 0x20), bridge(0x100, 0x12, 0x12)];
    let path = ancestry(&functions, 0, 0x1200, &[0]).unwrap();
    assert_eq!(&path.bridges[..path.count], &[0x100, 0x08]);
    assert_eq!(ancestry(&functions, 0, 0x20, &[0]).unwrap().count, 0);
}
#[test]
fn ambiguous_missing_and_invalid_parent_routing_rejects() {
    assert!(matches!(
        ancestry(&[bridge(8, 1, 1), bridge(16, 1, 1)], 0, 0x100, &[0]),
        Err(Error::Ambiguous)
    ));
    assert!(matches!(ancestry(&[], 0, 0x1200, &[0]), Err(Error::Parent)));
    assert!(matches!(
        ancestry(&[bridge(0x200, 1, 1)], 0, 0x100, &[0]),
        Err(Error::Bridge)
    ));
    assert!(matches!(
        ancestry(&[bridge(8, 1, 0)], 0, 0x100, &[0]),
        Err(Error::Bridge)
    ));
}

#[test]
fn pri_and_optional_acs_vector_spans_reject_overlap_and_truncation() {
    for next in [0x108, 0x10c] {
        assert_eq!(
            extensions(|at| Ok(if at == 0x100 {
                header(0x13, next)
            } else if at == next {
                header(1, 0)
            } else {
                0
            })),
            Err(Error::Chain)
        );
    }
    assert_eq!(
        extensions(|at| Ok(if at == 0x100 {
            header(1, 0xff4)
        } else if at == 0xff4 {
            header(0x13, 0)
        } else {
            0
        })),
        Err(Error::Chain)
    );
    // ACS EC with size zero owns 8+32bytes, so a following header at108 overlaps.
    assert_eq!(
        extensions(|at| Ok(if at == 0x100 {
            header(0x0d, 0x108)
        } else if at == 0x104 {
            1 << 5
        } else {
            header(1, 0)
        })),
        Err(Error::Chain)
    );
    assert_eq!(
        extensions(|at| Ok(if at == 0x100 {
            header(1, 0xfe0)
        } else if at == 0xfe0 {
            header(0x0d, 0)
        } else if at == 0xfe4 {
            1 << 5
        } else {
            0
        })),
        Err(Error::Chain)
    );
    // Without EC support the undefined size field does not create a vector.
    assert!(extensions(|at| Ok(if at == 0x100 {
        header(0x0d, 0x108)
    } else if at == 0x104 {
        0xff00
    } else {
        header(1, 0)
    }))
    .is_ok());
    // A one-bit vector occupies a full DWORD at108.
    assert!(extensions(|at| Ok(if at == 0x100 {
        header(0x0d, 0x10c)
    } else if at == 0x104 {
        0x120
    } else if at == 0x10c {
        header(1, 0)
    } else {
        0
    }))
    .is_ok());
}

#[test]
fn every_ancestor_must_forward_the_observed_requester_bus() {
    let invalid = [bridge(0x08, 1, 1), bridge(0x100, 0x12, 0x12)];
    assert!(matches!(
        ancestry(&invalid, 0, 0x1200, &[0]),
        Err(Error::Bridge)
    ));
    let valid = [bridge(0x08, 1, 0x12), bridge(0x100, 0x12, 0x12)];
    assert_eq!(ancestry(&valid, 0, 0x1200, &[0]).unwrap().count, 2);
}
