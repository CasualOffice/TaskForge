//! Lexicographic board ranks (ADR-013).
//!
//! `task.position` is a **string**, not a float and not an integer. `docs/26`
//! §Board ordering: floats "run out of precision after ~50 drags between the
//! same pair of cards, at which point ordering silently corrupts", and integers
//! "require renumbering the column on every insert".
//!
//! # Only the append case is here
//!
//! [`appended`] is what creating a task needs, and creating a task is what
//! C-008's first half does. Generating a midpoint is what *dragging* needs, and
//! nothing drags yet — so it is not here, and neither is the compaction job
//! `docs/26` describes. What **is** guaranteed here is that a midpoint will
//! always exist when that code arrives: see
//! `a_midpoint_is_always_available_between_two_appended_ranks`.
//!
//! # `'0'` is reserved, and that is the whole trick
//!
//! The alphabet's smallest character never appears in a rank this module
//! produces. That is what leaves room below every rank: with a trailing `'0'`
//! there is *nothing* lexicographically between `"a0"` and `"a"`, because any
//! longer string sharing the prefix sorts after. Reserving the minimum
//! character is the difference between "insert between any pair" and "insert
//! between any pair except these".

/// The rank alphabet: ASCII digits then lowercase letters, in byte order.
///
/// Byte order **is** the sort order, which is what lets PostgreSQL order a
/// column of these with a plain B-tree: every character sorts identically under
/// every collation, so a database with a different `LC_COLLATE` orders a board
/// the same way.
const ALPHABET: &[u8] = b"0123456789abcdefghijklmnopqrstuvwxyz";

/// The digits [`appended`] may use — everything except the smallest character.
///
/// See the module docs. Base 35 rather than 36 costs 3% of the address space
/// and buys the property the whole scheme exists for.
const DIGITS: &[u8] = ALPHABET.split_at(1).1;

/// The fixed width of an appended rank.
///
/// Eight base-35 digits is 35^8 ≈ 2.2 × 10^12 cards in one column, against a
/// documented ceiling of 2,000,000 tasks in an entire workspace. Fixed width is
/// what makes ordinal order and lexicographic order the same order: `"9"` sorts
/// after `"10"`; `"00000009"` does not.
const WIDTH: usize = 8;

/// The rank for the `n`th card appended to a column, `n` counting from 1.
///
/// Task numbers are allocated in order within a project (ADR-008), so passing
/// the task's number ranks every new card after every earlier one **without
/// reading the column first** — one fewer query on the create path, and no lock
/// on a neighbouring row.
#[must_use]
pub fn appended(n: i64) -> String {
    let base = i64::try_from(DIGITS.len()).unwrap_or(35);
    let mut digits = [DIGITS[0]; WIDTH];
    let mut value = n.max(0);
    for slot in digits.iter_mut().rev() {
        *slot = DIGITS[usize::try_from(value % base).unwrap_or(0)];
        value /= base;
    }
    String::from_utf8_lossy(&digits).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn appended_ranks_sort_in_creation_order() {
        // The property the create path depends on. Variable-width base-35 would
        // break it at every power of 35 — "9" sorts after "10" — which is why
        // the width is fixed.
        let ranks: Vec<String> = (1..5000).map(appended).collect();
        let mut sorted = ranks.clone();
        sorted.sort();
        assert_eq!(ranks, sorted);
    }

    #[test]
    fn appended_ranks_are_distinct() {
        let set: std::collections::HashSet<String> = (1..5000).map(appended).collect();
        assert_eq!(set.len(), 4999);
    }

    #[test]
    fn a_rank_never_uses_the_smallest_character() {
        // The reservation the module docs describe. A rank ending in the
        // minimum character has nothing below it, and the first drag onto that
        // gap would have nowhere to go.
        let smallest = ALPHABET[0] as char;
        for n in [1_i64, 34, 35, 36, 1225, 42_875, i64::from(u32::MAX)] {
            let rank = appended(n);
            assert!(
                !rank.contains(smallest),
                "appended({n}) = {rank} contains the reserved character"
            );
        }
    }

    #[test]
    fn a_midpoint_is_always_available_between_two_appended_ranks() {
        // ADR-013's actual promise: "lexicographic ranks insert between any pair
        // by generating a midpoint string". Dragging is not implemented yet, so
        // this asserts the *space* exists — which is the part a bad alphabet
        // would take away, silently, until the day someone dragged a card.
        for n in 1..2000 {
            let (low, high) = (appended(n), appended(n + 1));
            let mid = format!("{low}{}", ALPHABET[1] as char);
            assert!(low < mid && mid < high, "no room between {low} and {high}");
        }
    }

    #[test]
    fn the_alphabet_is_collation_independent() {
        // These ranks are ordered by PostgreSQL, whose collation is a
        // deployment setting. Restricting the alphabet to ASCII alphanumerics
        // is what makes byte order and collation order the same order.
        assert!(ALPHABET.iter().all(u8::is_ascii_alphanumeric));
        assert!(ALPHABET.windows(2).all(|w| w[0] < w[1]), "not sorted");
        assert!(DIGITS.iter().all(|d| ALPHABET.contains(d)));
        assert_eq!(DIGITS.len(), ALPHABET.len() - 1);
    }

    #[test]
    fn the_width_covers_the_documented_ceiling() {
        // docs/30: 2,000,000 tasks per workspace. A column that overflowed the
        // width would wrap and reorder the whole board.
        let ranks: Vec<String> = [1_i64, 2_000_000, 4_000_000]
            .into_iter()
            .map(appended)
            .collect();
        assert!(ranks[0] < ranks[1] && ranks[1] < ranks[2], "{ranks:?}");
        assert!(ranks.iter().all(|r| r.len() == WIDTH));
    }
}
