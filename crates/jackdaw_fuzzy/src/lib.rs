//! A fuzzy finder used by Jackdaw
//!
//! The matching is done by the [`FuzzyMatcher`] struct, which stores a list of items
//! which must implement the [`FuzzyItem`] trait.
//!
//! Example:
//!
//! ```
//! use jackdaw_fuzzy::{FuzzyMatcher, MatchedStr};
//!
//! let strings = vec![
//!     String::from("Hello world"),
//!     String::from("Hi there"),
//!     String::from("Hello there"),
//!     String::from("Some more text"),
//! ];
//!
//! let mut matcher = FuzzyMatcher::from_items(strings);
//! matcher.update_pattern("Hello");
//!
//! let mut matches = vec![];
//!
//! // Iterates over the matches with the higher scoring ones first
//! for matched in matcher.matches() {
//!     let score = matched.score; // How closely did it match?
//!     let index = matched.index; // The index of the underlying item
//!
//!     // A slice of `MatchedStr`s, which are the ranges of the item's text
//!     for segment in &matched.segments {
//!         let text = &segment.text;        // The text of this segment
//!         let is_match = segment.is_match; // Should this segment of the string be higlighted (did it match the input string)?
//!     }
//!
//!     matches.push((index, Vec::from(matched.segments)));
//! }
//!
//! assert_eq!(matches.len(), 2);
//!
//! assert_eq!(matches[0].0, 0);
//! assert_eq!(&matches[0].1, &[
//!     MatchedStr {
//!         // "Hello" is a part of the input
//!         text: "Hello".to_string(),
//!         is_match: true,
//!     },
//!     MatchedStr {
//!         // but " world" isn't
//!         text: " world".to_string(),
//!         is_match: false
//!     }
//! ]);
//!
//! assert_eq!(matches[1].0, 2);
//! assert_eq!(&matches[1].1, &[
//!     MatchedStr {
//!         text: "Hello".to_string(),
//!         is_match: true,
//!     },
//!     MatchedStr {
//!         text: " there".to_string(),
//!         is_match: false
//!     }
//! ]);
//! ```

use std::collections::HashSet;

use nucleo_matcher::pattern::{CaseMatching, Normalization, Pattern};
use nucleo_matcher::{Config, Matcher, Utf32String};

/// This trait must be implemented by any item used with a [`FuzzyMatcher`]
pub trait FuzzyItem {
    /// Gets the string that this item should be matched with
    fn get_text(&self) -> String;
}

impl<T: ToString> FuzzyItem for T {
    fn get_text(&self) -> String {
        self.to_string()
    }
}

/// The engine for fuzzy matching.
///
/// It contains a list of items, each of which must implement [`FuzzyItem`], and a pattern which
/// the items are matched against. To set the pattern, use [`update_pattern`](Self::update_pattern) or [`with_pattern`](Self::with_pattern)
#[derive(Debug, Clone)]
pub struct FuzzyMatcher<T: FuzzyItem> {
    items: Vec<T>,
    pattern: Pattern,
    matcher: Matcher,
}

impl<T: FuzzyItem> Default for FuzzyMatcher<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T: FuzzyItem> FuzzyMatcher<T> {
    /// Creates a new fuzzy matcher with no items and pattern
    pub fn new() -> Self {
        Self::from_items(std::iter::empty())
    }

    /// Creates a new fuzzy matcher with items from the given iterator
    pub fn from_items(items: impl IntoIterator<Item = T>) -> Self {
        Self {
            items: items.into_iter().collect::<Vec<_>>(),
            pattern: Pattern::parse("", CaseMatching::Smart, Normalization::Smart),
            matcher: Matcher::new(Config::DEFAULT),
        }
    }

    /// Sets the pattern that items are matched against, returning itself
    pub fn with_pattern(mut self, pattern: &str) -> Self {
        self.update_pattern(pattern);

        self
    }

    /// Updates the pattern that items are matched against
    pub fn update_pattern(&mut self, pattern: &str) {
        self.pattern
            .reparse(pattern, CaseMatching::Smart, Normalization::Smart);
    }

    /// Adds an item to the item list
    pub fn push_item(&mut self, item: T) {
        self.items.push(item);
    }

    /// Adds an iterator of items to the item list
    pub fn push_items(&mut self, items: impl IntoIterator<Item = T>) {
        self.items.extend(items);
    }

    /// Adds an item to the item list, returning itself
    pub fn with_item(mut self, item: T) -> Self {
        self.push_item(item);
        self
    }

    /// Adds an iterator of items to the item list, returning itself
    pub fn with_items(mut self, items: impl IntoIterator<Item = T>) -> Self {
        self.push_items(items);
        self
    }

    /// Gets a reference to the list of items
    pub fn items(&self) -> &[T] {
        &self.items
    }

    /// Compute and iterate over all the items that the pattern matches, sorted with the highest
    /// scoring items positioned first
    pub fn matches(&mut self) -> FuzzyMatches<'_> {
        let mut matches = Vec::with_capacity(self.items.len());

        for (index, item) in self.items.iter().enumerate() {
            let text = Utf32String::from(item.get_text());
            let score = self.pattern.score(text.slice(..), &mut self.matcher);
            let Some(score) = score else {
                // If the score is `None`, it doesn't match the pattern at all
                continue;
            };

            matches.push((score, text, index));
        }

        // Sort the matches in descending order
        matches.sort_by(|a, b| b.0.cmp(&a.0));

        FuzzyMatches {
            index: 0,
            pattern: &self.pattern,
            matcher: &mut self.matcher,
            matches: matches.into_boxed_slice(),
        }
    }
}

/// An iterator of matches by a [`FuzzyMatcher`]
pub struct FuzzyMatches<'a> {
    index: usize,
    pattern: &'a Pattern,
    matcher: &'a mut Matcher,
    matches: Box<[(u32, Utf32String, usize)]>,
}

impl<'a> Iterator for FuzzyMatches<'a> {
    type Item = Match;

    fn next(&mut self) -> Option<Self::Item> {
        let Some((score, str, index)) = self.matches.get(self.index) else {
            return None;
        };

        self.index += 1;

        let mut indices = vec![];

        let haystack = str.slice(..);
        self.pattern
            .indices(haystack, &mut self.matcher, &mut indices);

        let indices = indices.into_iter().collect::<HashSet<_>>();

        let mut matched_parts = vec![];
        let mut current_match = MatchedStr {
            text: String::new(),
            is_match: false,
        };

        for (index, char) in haystack.chars().enumerate() {
            let is_match = indices.contains(&(index as u32));
            if current_match.is_match != is_match {
                if current_match.text.len() > 0 {
                    matched_parts.push(current_match);
                }

                current_match = MatchedStr {
                    text: String::new(),
                    is_match,
                };
            }

            current_match.text.push(char);
        }

        if current_match.text.len() > 0 {
            matched_parts.push(current_match);
        }

        let item = Match {
            segments: matched_parts.into_boxed_slice(),
            score: *score,
            index: *index,
        };

        Some(item)
    }
}

/// A single item matched by a [`FuzzyMatcher`]
#[derive(Debug, PartialEq, Clone)]
pub struct Match {
    /// The segments of the matched string, see [`MatchedStr`]
    pub segments: Box<[MatchedStr]>,
    /// How well does the item match the input?
    pub score: u32,
    /// The index of the underlying item
    pub index: usize,
}

/// An invidiual segment of a [`Match`], which are intended to be used via
/// [`TextSpan`](bevy::prelude::TextSpan)s
#[derive(Debug, PartialEq, Clone)]
pub struct MatchedStr {
    /// The part of the string that this segment contains
    pub text: String,
    /// Does this segment match a part of the input string? (Which usually means that it will be highlighted)
    pub is_match: bool,
}
