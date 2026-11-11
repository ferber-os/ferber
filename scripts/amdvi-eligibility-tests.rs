#[path = "../shared/amdvi_eligibility.rs"]
mod eligibility;
use eligibility::{qemu_edu, Coverage, Entry, Error};

fn topology() -> [Entry; 7] {
    [
        (0x00, 0x8086, 0x29c0, 0x06, 0x00, 0x00),
        (0x08, 0x1022, 0x1419, 0x08, 0x06, 0x00),
        (0x20, 0x1234, 0x11e8, 0x00, 0xff, 0x00),
        (0xf8, 0x8086, 0x2918, 0x06, 0x01, 0x00),
        (0xfa, 0x8086, 0x2922, 0x01, 0x06, 0x01),
        (0xfb, 0x8086, 0x2930, 0x0c, 0x05, 0x00),
        (0x10, 0x1234, 0x1111, 0x03, 0x00, 0x00),
    ]
    .map(
        |(requester, vendor, device, class, subclass, interface)| Entry {
            segment: 0,
            requester,
            vendor,
            device,
            class,
            subclass,
            interface,
            header: 0,
            coverage: Some(Coverage {
                unit: 8,
                alias: requester,
            }),
            extensions_valid: true,
        },
    )
}

#[test]
fn exact_topology_and_only_exact_structural_exceptions() {
    let mut entries = topology();
    qemu_edu(&entries).unwrap();
    entries.reverse();
    qemu_edu(&entries).unwrap();
    entries.reverse();
    entries[0].coverage = None;
    entries[1].coverage = None;
    qemu_edu(&entries).unwrap();
    entries[0].device ^= 1;
    assert_eq!(qemu_edu(&entries), Err(Error::Topology(0)));
}
#[test]
fn unexpected_identity_or_function_cannot_replace_an_allowed_entry() {
    for change in 0..6 {
        let mut entries = topology();
        match change {
            0 => entries[2].requester = 0x28,
            1 => entries[2].vendor ^= 1,
            2 => entries[2].device ^= 1,
            3 => entries[2].class ^= 1,
            4 => entries[2].subclass ^= 1,
            _ => entries[2].interface ^= 1,
        }
        assert_eq!(
            qemu_edu(&entries),
            Err(Error::Topology(entries[2].requester))
        );
    }
}
#[test]
fn every_function_must_be_present_exactly_once() {
    let entries = topology();
    assert_eq!(qemu_edu(&entries[..6]), Err(Error::FunctionCount));
    let mut duplicate = entries;
    duplicate[5] = duplicate[4];
    assert_eq!(qemu_edu(&duplicate), Err(Error::DuplicateRequester(0xfa)));
    let mut extra = entries.to_vec();
    extra.push(entries[2]);
    assert_eq!(qemu_edu(&extra), Err(Error::FunctionCount));
}
#[test]
fn endpoints_require_coverage_and_the_exact_unit() {
    for index in 2..7 {
        let mut entries = topology();
        entries[index].coverage = None;
        assert_eq!(
            qemu_edu(&entries),
            Err(Error::MissingCoverage(entries[index].requester))
        );
        entries[index].coverage = Some(Coverage {
            unit: 9,
            alias: entries[index].requester,
        });
        assert_eq!(
            qemu_edu(&entries),
            Err(Error::WrongUnit(entries[index].requester))
        );
    }
}
#[test]
fn bridge_other_segment_and_downstream_bus_are_rejected() {
    for change in 0..3 {
        let mut entries = topology();
        match change {
            0 => entries[2].header = 1,
            1 => entries[2].segment = 1,
            _ => entries[2].requester |= 0x100,
        }
        assert_eq!(
            qemu_edu(&entries),
            Err(Error::Topology(entries[2].requester))
        );
    }
}
#[test]
fn aliases_cannot_merge_requesters_or_add_unsupported_translation() {
    let mut entries = topology();
    entries[2].coverage.as_mut().unwrap().alias = 8;
    assert_eq!(qemu_edu(&entries), Err(Error::SharedAlias(8)));
    entries[2].coverage.as_mut().unwrap().alias = 0x21;
    assert_eq!(qemu_edu(&entries), Err(Error::UnsupportedAlias(0x20)));
}
#[test]
fn bad_extensions_never_authorize_an_entry() {
    for index in 0..7 {
        let mut entries = topology();
        entries[index].extensions_valid = false;
        assert_eq!(
            qemu_edu(&entries),
            Err(Error::Extensions(entries[index].requester))
        );
    }
}
