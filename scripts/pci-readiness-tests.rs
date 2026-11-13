#[path = "../shared/acpi.rs"]
mod acpi;
#[path = "../ferber/src/arch/x86_64/platform/pci.rs"]
mod pci;
#[path = "../shared/pci_readiness.rs"]
mod readiness;
#[path = "../shared/acpi_tables.rs"]
mod tables;
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

#[test]
fn unknown_extension_snapshots_do_not_become_absent_or_complete() {
    // Actual MSI host bridge: vendor/device-looking DWORD, not a valid next pointer.
    let failed = extensions(|at| Ok(if at == 0x100 { 0x14d8_1022 } else { 0 }));
    assert_eq!(failed, Err(Error::Chain));
    let absent = extensions(|_| Ok(0));
    assert_eq!(absent, Ok(Extensions::default()));
    let enabled = Ok(Extensions {
        ats: Some(true),
        ..Extensions::default()
    });
    let mut summary = ExtensionSummary::default();
    summary.record(absent);
    summary.record(enabled);
    assert!(summary.complete());
    assert_eq!(summary.known_enabled, 1);
    summary.record(failed);
    summary.record(Err(Error::Read));
    summary.record(absent);
    assert_eq!(summary.unknown, 2);
    assert_eq!(summary.known_enabled, 1);
    assert!(!summary.complete());
}

#[test]
fn captured_failure_context_is_original_and_does_not_reread() {
    let mut reads = Vec::new();
    let captured = capture_extensions(|at| {
        reads.push(at);
        Ok(0x14d8_1022)
    });
    assert_eq!(reads, [0x100]);
    assert_eq!(captured.result, Err(Error::Chain));
    assert_eq!(captured.raw100, Some(Ok(0x14d8_1022)));
    assert_eq!(captured.last_read, Some((0x100, Ok(0x14d8_1022))));
    // Inspecting the cached context must not perform another configuration access.
    assert_eq!(captured.raw100, captured.last_read.map(|(_, value)| value));
    assert_eq!(reads, [0x100]);
    let failed = capture_extensions(|at| {
        if at == 0x100 {
            Ok(header(0x0f, 0))
        } else {
            Err(Error::Read)
        }
    });
    assert_eq!(failed.result, Err(Error::Read));
    assert_eq!(failed.raw100, Some(Ok(header(0x0f, 0))));
    assert_eq!(failed.last_read, Some((0x104, Err(Error::Read))));
    let unreadable = capture_extensions(|_| Err(Error::Read));
    assert_eq!(unreadable.raw100, Some(Err(Error::Read)));
    assert_eq!(unreadable.last_read, Some((0x100, Err(Error::Read))));
    let absent = capture_extensions(|_| Ok(0));
    assert_eq!(absent.result, Ok(Extensions::default()));
    assert_eq!(absent.raw100, Some(Ok(0)));
}

fn conventional(class: u8, subclass: u8, device: u16) -> [u8; 256] {
    let mut b = [0; 256];
    b[..4].copy_from_slice(&((device as u32) << 16 | 0x1022).to_le_bytes());
    b[11] = class;
    b[10] = subclass;
    b
}
fn capability(b: &mut [u8; 256], id: u8, at: u8) {
    b[6] = 0x10;
    b[0x34] = at;
    b[at as usize] = id;
}
#[test]
fn conventional_fch_functions_never_read_extended_configuration() {
    for b in [conventional(0x0c, 5, 0x790b), conventional(6, 1, 0x790e)] {
        let captured =
            capture_configuration(Ok(b), |_| panic!("extended read on conventional function"));
        assert_eq!(captured.space, ConfigSpace::ConventionalOnly);
        assert_eq!(captured.result, Ok(Extensions::default()));
        assert_eq!(captured.raw100, None);
    }
}
#[test]
fn host_pcie_and_pcix_classification_preserves_unknown_and_enabled_state() {
    let host = capture_configuration(Ok(conventional(6, 0, 0x14d8)), |_| Ok(0x14d81022));
    assert_eq!(host.space, ConfigSpace::Extended);
    assert_eq!(host.result, Err(Error::Chain));
    assert_eq!(host.raw100, Some(Ok(0x14d81022)));
    let mut pcie = conventional(2, 0, 1);
    capability(&mut pcie, 0x10, 0x40);
    let observed = capture_configuration(Ok(pcie), |at| {
        Ok(if at == 0x100 { header(0xf, 0) } else { 1 << 31 })
    });
    assert_eq!(observed.result.unwrap().ats, Some(true));
    let observed = capture_configuration(Ok(pcie), |at| {
        Ok(match at {
            0x100 => header(0xf, 0x108),
            0x104 => 1 << 31,
            0x108 => header(0x13, 0x118),
            0x10c => 1,
            0x118 => header(0x1b, 0),
            0x11c => 1 << 16,
            _ => 0,
        })
    });
    let value = observed.result.unwrap();
    assert_eq!(
        (value.ats, value.pri, value.pasid),
        (Some(true), Some(true), Some(true))
    );

    for bits in [0, 1 << 30, 1 << 31] {
        let mut pcix = conventional(2, 0, 1);
        capability(&mut pcix, 7, 0x40);
        pcix[0x44..0x48].copy_from_slice(&u32::to_le_bytes(bits));
        let mut reads = 0;
        let observed = capture_configuration(Ok(pcix), |_| {
            reads += 1;
            Ok(0)
        });
        assert_eq!(reads, usize::from(bits != 0));
        assert_eq!(
            observed.space,
            if bits == 0 {
                ConfigSpace::ConventionalOnly
            } else {
                ConfigSpace::Extended
            }
        );
    }
}
#[test]
fn conventional_errors_and_truncated_pcix_remain_unknown_without_extended_reads() {
    let mut cycle = conventional(2, 0, 1);
    capability(&mut cycle, 1, 0x40);
    cycle[0x41] = 0x40;
    let mut truncated = conventional(2, 0, 1);
    capability(&mut truncated, 7, 0xfc);
    let mut overlap = conventional(2, 0, 1);
    capability(&mut overlap, 7, 0x40);
    overlap[0x41] = 0x44;
    overlap[0x44] = 1;
    for bytes in [Err(Error::Read), Ok(cycle), Ok(truncated), Ok(overlap)] {
        let observed =
            capture_configuration(bytes, |_| panic!("extended read after invalid metadata"));
        assert_eq!(observed.space, ConfigSpace::Unknown);
        assert!(observed.result.is_err());
    }
    let mut pcie = conventional(2, 0, 1);
    capability(&mut pcie, 0x10, 0x40);
    let observed = capture_configuration(Ok(pcie), |_| Err(Error::Read));
    assert_eq!(observed.space, ConfigSpace::Extended);
    assert_eq!(observed.result, Err(Error::Read));
}

#[test]
fn qemu_iommu_dummy_msi_overlap_remains_unknown() {
    let mut bytes = conventional(8, 6, 0x1419);
    capability(&mut bytes, 5, 0x60);
    bytes[0x61] = 0x5c;
    bytes[0x62] = 0x80;
    bytes[0x5c..0x5e].copy_from_slice(&[8, 0x58]);
    bytes[0x58..0x5a].copy_from_slice(&[5, 0x40]);
    bytes[0x40] = 0x0f;
    let observed =
        capture_configuration(Ok(bytes), |_| panic!("extended read after overlapping MSI"));
    assert_eq!(observed.space, ConfigSpace::Unknown);
    assert_eq!(observed.result, Err(Error::Chain));
}
