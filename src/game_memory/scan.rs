//! Searches over bytes copied out of the game.

use core::ops::RangeInclusive;

use memchr::memmem;
use valveprotos::deadlock::CMsgClientToGcGetMatchMetaDataResponse;
use valveprotos::deadlock::c_msg_client_to_gc_get_match_meta_data_response::EResult;

use crate::utils::MAX_MATCH_ID;

fn u32_at(bytes: &[u8], at: usize) -> Option<u32> {
    bytes
        .get(at..)?
        .first_chunk()
        .copied()
        .map(u32::from_le_bytes)
}

/// The reply object is twelve 32-bit words: a vtable pointer, eight zero bytes of internal
/// metadata, has-bits, a zero cached size, then the six proto fields in order. Words 2 to 6
/// are always the same for a successful reply with every field set.
const REPLY_SIZE: usize = 48;
const REPLY_FIXED: [u8; 20] = [
    0, 0, 0, 0, 0, 0, 0, 0, // internal metadata
    0x3f, 0, 0, 0, // has-bits: all six fields
    0, 0, 0, 0, // cached size
    1, 0, 0, 0, // result: success
];
const REPLY_FIXED_AT: usize = 8;
/// Unix times the reply could hold (2020 to 2038).
const UNIX_TIMES: RangeInclusive<u32> = 1_577_836_800..=2_145_916_800;

/// Every successful `GetMatchMetaData` reply in `buf`, which starts at address `base`.
pub(super) fn metadata_replies(
    buf: &[u8],
    base: usize,
) -> impl Iterator<Item = CMsgClientToGcGetMatchMetaDataResponse> {
    memmem::find_iter(buf, &REPLY_FIXED).filter_map(move |hit| {
        let at = hit.checked_sub(REPLY_FIXED_AT)?;
        // The heap aligns the object to 16 bytes.
        if !base.wrapping_add(at).is_multiple_of(16) {
            return None;
        }
        reply_from(buf.get(at..)?.first_chunk()?)
    })
}

fn reply_from(bytes: &[u8; REPLY_SIZE]) -> Option<CMsgClientToGcGetMatchMetaDataResponse> {
    let words: [[u8; 4]; 12] = bytes.as_chunks().0.try_into().ok()?;
    let [
        _,
        vtable_high,
        ..,
        replay_salt,
        metadata_salt,
        valid_through,
        group_id,
        processing_through,
    ] = words.map(u32::from_le_bytes);
    // A 64-bit pointer into a loaded module.
    let plausible = (0x7ff0..=0x7fff).contains(&vtable_high)
        && metadata_salt != 0
        && (1..=4000).contains(&group_id)
        && UNIX_TIMES.contains(&valid_through)
        && UNIX_TIMES.contains(&processing_through)
        && valid_through > processing_through;
    plausible.then(|| CMsgClientToGcGetMatchMetaDataResponse {
        result: Some(EResult::KEResultSuccess.into()),
        replay_salt: Some(replay_salt),
        metadata_salt: Some(metadata_salt),
        replay_valid_through: Some(valid_through),
        replay_group_id: Some(group_id),
        replay_processing_through: Some(processing_through),
    })
}

/// One entry of the game's match history list.
const RECORD_SIZE: usize = 144;
const MIN_MATCH_ID: u64 = 1_000_000;
/// 1 Jan 2024, before Deadlock's first public match.
const FIRST_MATCH_START: u32 = 1_704_067_200;
const DAY_SECS: u32 = 24 * 60 * 60;

/// `(start time, match id)` of the history record at `at`, when every field is one a real
/// match could have.
fn record_at(buf: &[u8], at: usize, now: u32) -> Option<(u32, u64)> {
    let record: &[u8; RECORD_SIZE] = buf.get(at..)?.first_chunk()?;
    let match_id = u64::from_le_bytes(*record.first_chunk()?);
    if !(MIN_MATCH_ID..=MAX_MATCH_ID).contains(&match_id) {
        return None;
    }
    let within = |offset: usize, range: RangeInclusive<u32>| {
        u32_at(record, offset).is_some_and(|v| range.contains(&v))
    };
    let start = u32_at(record, 16)?;
    let plausible = within(8, 1..=128) // hero id
        && within(12, 1..=3 * 60 * 60) // duration
        && (FIRST_MATCH_START..=now.saturating_add(DAY_SECS)).contains(&start)
        && within(20, 0..=1) // winning team
        && within(24, 0..=1) // player team
        && [28, 32, 36].into_iter().all(|kda| within(kda, 0..=200))
        && within(40, 0..=5_000) // last hits
        && within(44, 0..=5_000) // denies
        && within(48, 0..=100) // hero level
        && within(52, 0..=1_000_000) // net worth
        && within(76, 1..=16) // match mode
        && within(80, 0..=16) // game mode
        && within(100, 0..=3); // outcome
    plausible.then_some((start, match_id))
}

/// `(start time, match id)` of every match in the histories in `buf`: the player's own and
/// any opened from a profile. A history is records back to back, newest first, so a record
/// only counts next to another one in that order.
pub(super) fn history_matches(buf: &[u8], now: u32) -> Vec<(u32, u64)> {
    let mut out = Vec::new();
    let mut at = 0;
    while at < buf.len() {
        let in_run = record_at(buf, at, now).filter(|&(start, id)| {
            let older_after = at
                .checked_add(RECORD_SIZE)
                .and_then(|next| record_at(buf, next, now))
                .is_some_and(|(next_start, next_id)| next_id != id && next_start <= start);
            let newer_before = at
                .checked_sub(RECORD_SIZE)
                .and_then(|prev| record_at(buf, prev, now))
                .is_some_and(|(prev_start, prev_id)| prev_id != id && prev_start >= start);
            older_after || newer_before
        });
        let step = match in_run {
            Some(found) => {
                out.push(found);
                RECORD_SIZE
            }
            None => 8,
        };
        at = at.saturating_add(step);
    }
    out
}

#[cfg(test)]
#[allow(clippy::indexing_slicing, clippy::arithmetic_side_effects)]
mod tests {
    use super::*;

    const NOW: u32 = 1_789_080_000;

    fn words(words: &[u32]) -> Vec<u8> {
        words.iter().flat_map(|w| w.to_le_bytes()).collect()
    }

    /// A reply read from the live game for match 102948911.
    const LIVE_REPLY: [u32; 12] = [
        603_464_808,
        32763,
        0,
        0,
        63,
        0,
        1,
        509_659_246,
        818_291_387,
        1_790_037_025,
        273,
        1_788_224_474,
    ];

    #[test]
    fn reads_the_reply() {
        let mut buf = vec![0; 32];
        buf.extend(words(&LIVE_REPLY));
        buf.extend([0; 16]);
        let found: Vec<_> = metadata_replies(&buf, 0x10).collect();
        assert_eq!(found.len(), 1);
        assert_eq!(
            (
                found[0].replay_group_id(),
                found[0].replay_salt(),
                found[0].metadata_salt(),
                found[0].replay_valid_through(),
            ),
            (273, 509_659_246, 818_291_387, 1_790_037_025)
        );
        assert_eq!(
            metadata_replies(&buf, 0x18).count(),
            0,
            "a misaligned copy is not the object"
        );
    }

    #[test]
    fn a_failed_or_partial_reply_is_not_read() {
        for (word, value) in [(6, 7), (4, 0b111), (10, 0), (1, 0)] {
            let mut reply = LIVE_REPLY;
            reply[word] = value;
            assert_eq!(metadata_replies(&words(&reply), 0).count(), 0);
        }
    }

    fn record(match_id: u64, start_time: u32) -> Vec<u8> {
        let mut fields = [0_u32; RECORD_SIZE / 4];
        fields[2] = 7; // hero
        fields[3] = 1845; // duration
        fields[4] = start_time;
        fields[6] = 1; // player team
        fields[7..10].copy_from_slice(&[5, 4, 4]);
        fields[10] = 204; // last hits
        fields[12] = 32; // level
        fields[13] = 36_395; // net worth
        fields[19] = 4; // match mode
        fields[20] = 1; // game mode
        fields[25] = 2; // outcome
        let mut bytes = words(&fields);
        bytes[..8].copy_from_slice(&match_id.to_le_bytes());
        bytes
    }

    #[test]
    fn history_records_count_only_in_a_run() {
        let mut buf = vec![0xAB; 1000];
        buf.extend(record(104_921_839, 1_789_078_989));
        buf.extend(record(104_910_270, 1_789_070_000));
        buf.extend([0x11; 333]);
        assert_eq!(
            history_matches(&buf, NOW),
            [(1_789_078_989, 104_921_839), (1_789_070_000, 104_910_270)]
        );
        let alone = record(104_921_839, 1_789_078_989);
        assert!(
            history_matches(&alone, NOW).is_empty(),
            "one record alone is noise"
        );
        assert!(
            history_matches(&buf, 1_700_000_000).is_empty(),
            "a start in the future is not a match"
        );
    }
}
