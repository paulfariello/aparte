/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at http://mozilla.org/MPL/2.0/. */

/// A vim-style motion for Normal mode: where the cursor moves, or the range an
/// operator covers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Motion {
    // ── Text motions (operate on the input-bar TextEditor) ──────────────────
    Left,            // h
    Right,           // l — capped at len-1 in Normal mode
    WordForward,     // w
    WordBackward,    // b
    WordEnd,         // e  (inclusive)
    BigWordForward,  // W
    BigWordBackward, // B
    BigWordEnd,      // E  (inclusive)
    LineStart,       // 0
    FirstNonBlank,   // ^
    LineEnd,         // $  (inclusive)
    WholeLine,       // produced by dd / cc / yy
    AWord,           // aw — word + adjacent whitespace (inclusive)
    IWord,           // iw — inner word only (not inclusive)
    PasteAfter,      // p
    PasteBefore,     // P
    // ── Navigation motions (operate on the ScrollWin, not the text) ─────────
    Up,         // k
    Down,       // j
    FileTop,    // gg
    FileBottom, // G
    SearchNext, // n
    SearchPrev, // N
}

impl Motion {
    /// Whether the character *at* the motion target is included in the
    /// operator range (vim "inclusive" motions).
    pub fn is_inclusive(&self) -> bool {
        matches!(
            self,
            Motion::WordEnd | Motion::BigWordEnd | Motion::LineEnd | Motion::AWord
        )
    }

    /// Whether this motion drives ScrollWin navigation rather than editing.
    pub fn is_navigation(&self) -> bool {
        matches!(
            self,
            Motion::Up
                | Motion::Down
                | Motion::FileTop
                | Motion::FileBottom
                | Motion::SearchNext
                | Motion::SearchPrev
        )
    }
}

/// The operator half of a vim action.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Operator {
    Move,
    Delete,
    Change,
    Yank,
}

/// A fully parsed vim Normal-mode action.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Action {
    /// Repetition count (defaults to 1 when omitted).
    pub count: usize,
    /// Named register (`None` → unnamed register `"`).
    pub register: Option<char>,
    pub operator: Operator,
    pub motion: Motion,
}

/// Result returned by `ActionParser::feed`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ParseResult {
    /// Sequence is a valid prefix; more characters expected.
    Pending,
    /// A complete action has been parsed.
    Complete(Action),
    /// The sequence cannot form a valid action.
    Invalid,
}

// ── Internal parser states ────────────────────────────────────────────────────

#[derive(Clone, Debug)]
enum ParserState {
    /// Counting digits before anything else.
    Count(usize),
    /// Just saw `"` — waiting for the register name character.
    WaitRegisterName { pre_count: usize },
    /// Have `"x` — waiting for an operator or motion.
    AfterRegister { pre_count: usize, reg: char },
    /// Have an operator (`d`/`c`/`y`) — waiting for a motion or count.
    AfterOperator {
        pre_count: usize,
        reg: Option<char>,
        op: Operator,
    },
    /// Have operator + count digits — waiting for the final motion.
    AfterOperatorCount {
        pre_count: usize,
        reg: Option<char>,
        op: Operator,
        op_count: usize,
    },
    /// Just saw `g` — waiting for the second character (only `gg` supported).
    GPrefix { pre_count: usize, reg: Option<char> },
    /// Have operator + `a`/`i` — waiting for the text-object character (e.g. `w`).
    AfterTextObjectPrefix {
        pre_count: usize,
        reg: Option<char>,
        op: Operator,
        around: bool,
    },
}

// ── ActionParser ──────────────────────────────────────────────────────────────

/// Parses vim Normal-mode key sequences into [`Action`] values.
///
/// Grammar: `[count]["register][operator][count][motion]`
///
/// After `feed` returns [`ParseResult::Complete`] or [`ParseResult::Invalid`]
/// the parser automatically resets; callers do not need to call `reset()`
/// themselves unless they want to abort a pending sequence early.
#[derive(Debug, Default)]
pub struct ActionParser {
    state: Option<ParserState>,
    buffer: String,
}

impl ActionParser {
    pub fn new() -> Self {
        Self::default()
    }

    /// Abandon any pending sequence and reset to the initial state.
    pub fn reset(&mut self) {
        self.state = None;
        self.buffer.clear();
    }

    /// Characters accumulated since the last reset — suitable for
    /// `CommandBufferUpdate` display.
    pub fn pending(&self) -> &str {
        &self.buffer
    }

    pub fn is_pending(&self) -> bool {
        self.state.is_some()
    }

    /// Feed one character. Returns whether the sequence is still pending,
    /// complete, or invalid.  On `Complete` or `Invalid` the parser resets
    /// itself automatically.
    pub fn feed(&mut self, c: char) -> ParseResult {
        self.buffer.push(c);
        let state = self.state.take().unwrap_or(ParserState::Count(0));
        let result = self.transition(state, c);
        if matches!(result, ParseResult::Complete(_) | ParseResult::Invalid) {
            self.reset();
        }
        result
    }

    // ── helpers ──────────────────────────────────────────────────────────────

    fn done(&self, count: usize, reg: Option<char>, op: Operator, motion: Motion) -> ParseResult {
        ParseResult::Complete(Action {
            count,
            register: reg,
            operator: op,
            motion,
        })
    }

    fn transition(&mut self, state: ParserState, c: char) -> ParseResult {
        match state {
            ParserState::Count(0) => self.on_init(1, None, c),
            ParserState::Count(n) => self.on_count(n, c),
            ParserState::WaitRegisterName { pre_count } => self.on_wait_register(pre_count, c),
            ParserState::AfterRegister { pre_count, reg } => {
                self.on_after_register(pre_count, reg, c)
            }
            ParserState::AfterOperator { pre_count, reg, op } => {
                self.on_after_operator(pre_count, reg, op, c)
            }
            ParserState::AfterOperatorCount {
                pre_count,
                reg,
                op,
                op_count,
            } => self.on_after_operator_count(pre_count, reg, op, op_count, c),
            ParserState::GPrefix { pre_count, reg } => self.on_g_prefix(pre_count, reg, c),
            ParserState::AfterTextObjectPrefix {
                pre_count,
                reg,
                op,
                around,
            } => self.on_after_text_object_prefix(pre_count, reg, op, around, c),
        }
    }

    /// Process `c` given the accumulated `pre_count` and optional `reg`.
    fn on_init(&mut self, pre_count: usize, reg: Option<char>, c: char) -> ParseResult {
        match c {
            '1'..='9' => {
                // Start of a count (we also handle the single-digit case here
                // so we can reuse this function from on_count).
                self.state = Some(ParserState::Count(c as usize - '0' as usize));
                ParseResult::Pending
            }
            '"' => {
                self.state = Some(ParserState::WaitRegisterName { pre_count });
                ParseResult::Pending
            }
            'd' => {
                self.state = Some(ParserState::AfterOperator {
                    pre_count,
                    reg,
                    op: Operator::Delete,
                });
                ParseResult::Pending
            }
            'c' => {
                self.state = Some(ParserState::AfterOperator {
                    pre_count,
                    reg,
                    op: Operator::Change,
                });
                ParseResult::Pending
            }
            'y' => {
                self.state = Some(ParserState::AfterOperator {
                    pre_count,
                    reg,
                    op: Operator::Yank,
                });
                ParseResult::Pending
            }
            'g' => {
                self.state = Some(ParserState::GPrefix { pre_count, reg });
                ParseResult::Pending
            }
            'x' => self.done(pre_count, reg, Operator::Delete, Motion::Right),
            'X' => self.done(pre_count, reg, Operator::Delete, Motion::Left),
            'p' => self.done(pre_count, reg, Operator::Move, Motion::PasteAfter),
            'P' => self.done(pre_count, reg, Operator::Move, Motion::PasteBefore),
            _ => match char_to_motion(c) {
                Some(m) => self.done(pre_count, reg, Operator::Move, m),
                None => ParseResult::Invalid,
            },
        }
    }

    fn on_count(&mut self, n: usize, c: char) -> ParseResult {
        match c {
            '0'..='9' => {
                self.state = Some(ParserState::Count(n * 10 + (c as usize - '0' as usize)));
                ParseResult::Pending
            }
            _ => self.on_init(n, None, c),
        }
    }

    fn on_wait_register(&mut self, pre_count: usize, c: char) -> ParseResult {
        if c.is_ascii_alphabetic() || c == '"' || c == '+' || c == '*' || c == '-' {
            self.state = Some(ParserState::AfterRegister { pre_count, reg: c });
            ParseResult::Pending
        } else {
            ParseResult::Invalid
        }
    }

    fn on_after_register(&mut self, pre_count: usize, reg: char, c: char) -> ParseResult {
        self.on_init(pre_count, Some(reg), c)
    }

    fn on_after_operator(
        &mut self,
        pre_count: usize,
        reg: Option<char>,
        op: Operator,
        c: char,
    ) -> ParseResult {
        // Operator doubling: dd / cc / yy → WholeLine.
        let op_char = match op {
            Operator::Delete => 'd',
            Operator::Change => 'c',
            Operator::Yank => 'y',
            Operator::Move => unreachable!(),
        };
        if c == op_char {
            return self.done(pre_count, reg, op, Motion::WholeLine);
        }
        match c {
            '1'..='9' => {
                self.state = Some(ParserState::AfterOperatorCount {
                    pre_count,
                    reg,
                    op,
                    op_count: c as usize - '0' as usize,
                });
                ParseResult::Pending
            }
            'a' => {
                self.state = Some(ParserState::AfterTextObjectPrefix {
                    pre_count,
                    reg,
                    op,
                    around: true,
                });
                ParseResult::Pending
            }
            'i' => {
                self.state = Some(ParserState::AfterTextObjectPrefix {
                    pre_count,
                    reg,
                    op,
                    around: false,
                });
                ParseResult::Pending
            }
            _ => match char_to_motion(c) {
                Some(m) => self.done(pre_count, reg, op, m),
                None => ParseResult::Invalid,
            },
        }
    }

    fn on_after_text_object_prefix(
        &mut self,
        pre_count: usize,
        reg: Option<char>,
        op: Operator,
        around: bool,
        c: char,
    ) -> ParseResult {
        match c {
            'w' => self.done(
                pre_count,
                reg,
                op,
                if around { Motion::AWord } else { Motion::IWord },
            ),
            _ => ParseResult::Invalid,
        }
    }

    fn on_after_operator_count(
        &mut self,
        pre_count: usize,
        reg: Option<char>,
        op: Operator,
        op_count: usize,
        c: char,
    ) -> ParseResult {
        match c {
            '0'..='9' => {
                self.state = Some(ParserState::AfterOperatorCount {
                    pre_count,
                    reg,
                    op,
                    op_count: op_count * 10 + (c as usize - '0' as usize),
                });
                ParseResult::Pending
            }
            _ => match char_to_motion(c) {
                // vim: [n1][op][n2][motion] → total count = n1 * n2
                Some(m) => self.done(pre_count * op_count, reg, op, m),
                None => ParseResult::Invalid,
            },
        }
    }

    fn on_g_prefix(&mut self, pre_count: usize, reg: Option<char>, c: char) -> ParseResult {
        match c {
            'g' => self.done(pre_count, reg, Operator::Move, Motion::FileTop),
            _ => ParseResult::Invalid,
        }
    }
}

fn char_to_motion(c: char) -> Option<Motion> {
    match c {
        'h' => Some(Motion::Left),
        'l' => Some(Motion::Right),
        'w' => Some(Motion::WordForward),
        'b' => Some(Motion::WordBackward),
        'e' => Some(Motion::WordEnd),
        'W' => Some(Motion::BigWordForward),
        'B' => Some(Motion::BigWordBackward),
        'E' => Some(Motion::BigWordEnd),
        '0' => Some(Motion::LineStart),
        '^' => Some(Motion::FirstNonBlank),
        '$' => Some(Motion::LineEnd),
        'j' => Some(Motion::Down),
        'k' => Some(Motion::Up),
        'G' => Some(Motion::FileBottom),
        'n' => Some(Motion::SearchNext),
        'N' => Some(Motion::SearchPrev),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn feed_all(seq: &str) -> ParseResult {
        let mut parser = ActionParser::new();
        let mut result = ParseResult::Invalid;
        for c in seq.chars() {
            result = parser.feed(c);
        }
        result
    }

    fn complete(
        count: usize,
        register: Option<char>,
        operator: Operator,
        motion: Motion,
    ) -> ParseResult {
        ParseResult::Complete(Action {
            count,
            register,
            operator,
            motion,
        })
    }

    // ── Single motions (no operator) ────────────────────────────────────────

    #[test]
    fn single_motion_w() {
        assert_eq!(
            feed_all("w"),
            complete(1, None, Operator::Move, Motion::WordForward)
        );
    }

    #[test]
    fn single_motion_dollar() {
        assert_eq!(
            feed_all("$"),
            complete(1, None, Operator::Move, Motion::LineEnd)
        );
    }

    #[test]
    fn single_motion_gg() {
        assert_eq!(
            feed_all("gg"),
            complete(1, None, Operator::Move, Motion::FileTop)
        );
    }

    #[test]
    fn single_motion_big_g() {
        assert_eq!(
            feed_all("G"),
            complete(1, None, Operator::Move, Motion::FileBottom)
        );
    }

    #[test]
    fn single_motion_h() {
        assert_eq!(
            feed_all("h"),
            complete(1, None, Operator::Move, Motion::Left)
        );
    }

    #[test]
    fn single_motion_l() {
        assert_eq!(
            feed_all("l"),
            complete(1, None, Operator::Move, Motion::Right)
        );
    }

    #[test]
    fn single_motion_b() {
        assert_eq!(
            feed_all("b"),
            complete(1, None, Operator::Move, Motion::WordBackward)
        );
    }

    #[test]
    fn single_motion_e() {
        assert_eq!(
            feed_all("e"),
            complete(1, None, Operator::Move, Motion::WordEnd)
        );
    }

    #[test]
    fn single_motion_zero() {
        assert_eq!(
            feed_all("0"),
            complete(1, None, Operator::Move, Motion::LineStart)
        );
    }

    #[test]
    fn single_motion_caret() {
        assert_eq!(
            feed_all("^"),
            complete(1, None, Operator::Move, Motion::FirstNonBlank)
        );
    }

    #[test]
    fn single_motion_j() {
        assert_eq!(
            feed_all("j"),
            complete(1, None, Operator::Move, Motion::Down)
        );
    }

    #[test]
    fn single_motion_k() {
        assert_eq!(feed_all("k"), complete(1, None, Operator::Move, Motion::Up));
    }

    #[test]
    fn single_motion_n() {
        assert_eq!(
            feed_all("n"),
            complete(1, None, Operator::Move, Motion::SearchNext)
        );
    }

    #[test]
    fn single_motion_big_n() {
        assert_eq!(
            feed_all("N"),
            complete(1, None, Operator::Move, Motion::SearchPrev)
        );
    }

    // ── Count prefix ────────────────────────────────────────────────────────

    #[test]
    fn count_3_w() {
        assert_eq!(
            feed_all("3w"),
            complete(3, None, Operator::Move, Motion::WordForward)
        );
    }

    #[test]
    fn count_12_j() {
        assert_eq!(
            feed_all("12j"),
            complete(12, None, Operator::Move, Motion::Down)
        );
    }

    #[test]
    fn count_5_h() {
        assert_eq!(
            feed_all("5h"),
            complete(5, None, Operator::Move, Motion::Left)
        );
    }

    // ── Operators + motions ─────────────────────────────────────────────────

    #[test]
    fn operator_delete_word_forward() {
        assert_eq!(
            feed_all("dw"),
            complete(1, None, Operator::Delete, Motion::WordForward)
        );
    }

    #[test]
    fn operator_delete_line_end() {
        assert_eq!(
            feed_all("d$"),
            complete(1, None, Operator::Delete, Motion::LineEnd)
        );
    }

    #[test]
    fn operator_delete_line_start() {
        assert_eq!(
            feed_all("d0"),
            complete(1, None, Operator::Delete, Motion::LineStart)
        );
    }

    #[test]
    fn operator_change_word() {
        assert_eq!(
            feed_all("cw"),
            complete(1, None, Operator::Change, Motion::WordForward)
        );
    }

    #[test]
    fn operator_yank_word() {
        assert_eq!(
            feed_all("yw"),
            complete(1, None, Operator::Yank, Motion::WordForward)
        );
    }

    // ── Count + operator + motion ───────────────────────────────────────────

    #[test]
    fn pre_count_3_dw() {
        assert_eq!(
            feed_all("3dw"),
            complete(3, None, Operator::Delete, Motion::WordForward)
        );
    }

    #[test]
    fn op_count_d3w() {
        assert_eq!(
            feed_all("d3w"),
            complete(3, None, Operator::Delete, Motion::WordForward)
        );
    }

    #[test]
    fn pre_and_op_count_2d3w() {
        assert_eq!(
            feed_all("2d3w"),
            complete(6, None, Operator::Delete, Motion::WordForward)
        );
    }

    // ── Operator doubling (whole-line) ──────────────────────────────────────

    #[test]
    fn operator_dd() {
        assert_eq!(
            feed_all("dd"),
            complete(1, None, Operator::Delete, Motion::WholeLine)
        );
    }

    #[test]
    fn operator_yy() {
        assert_eq!(
            feed_all("yy"),
            complete(1, None, Operator::Yank, Motion::WholeLine)
        );
    }

    #[test]
    fn operator_cc() {
        assert_eq!(
            feed_all("cc"),
            complete(1, None, Operator::Change, Motion::WholeLine)
        );
    }

    #[test]
    fn count_2_dd() {
        assert_eq!(
            feed_all("2dd"),
            complete(2, None, Operator::Delete, Motion::WholeLine)
        );
    }

    // ── Register prefix ─────────────────────────────────────────────────────

    #[test]
    fn register_a_dw() {
        assert_eq!(
            feed_all("\"adw"),
            complete(1, Some('a'), Operator::Delete, Motion::WordForward)
        );
    }

    #[test]
    fn register_upper_a_yy() {
        assert_eq!(
            feed_all("\"Ayy"),
            complete(1, Some('A'), Operator::Yank, Motion::WholeLine)
        );
    }

    #[test]
    fn register_unnamed_yw() {
        assert_eq!(
            feed_all("\"\"yw"),
            complete(1, Some('"'), Operator::Yank, Motion::WordForward)
        );
    }

    // ── Shorthands ──────────────────────────────────────────────────────────

    #[test]
    fn shorthand_x_deletes_right() {
        assert_eq!(
            feed_all("x"),
            complete(1, None, Operator::Delete, Motion::Right)
        );
    }

    #[test]
    fn shorthand_upper_x_deletes_left() {
        assert_eq!(
            feed_all("X"),
            complete(1, None, Operator::Delete, Motion::Left)
        );
    }

    #[test]
    fn count_3_x() {
        assert_eq!(
            feed_all("3x"),
            complete(3, None, Operator::Delete, Motion::Right)
        );
    }

    // ── Paste ────────────────────────────────────────────────────────────────

    #[test]
    fn paste_after() {
        assert_eq!(
            feed_all("p"),
            complete(1, None, Operator::Move, Motion::PasteAfter)
        );
    }

    #[test]
    fn paste_before() {
        assert_eq!(
            feed_all("P"),
            complete(1, None, Operator::Move, Motion::PasteBefore)
        );
    }

    #[test]
    fn register_paste_after() {
        assert_eq!(
            feed_all("\"ap"),
            complete(1, Some('a'), Operator::Move, Motion::PasteAfter)
        );
    }

    // ── Pending states ───────────────────────────────────────────────────────

    #[test]
    fn pending_after_operator_d() {
        assert_eq!(feed_all("d"), ParseResult::Pending);
    }

    #[test]
    fn pending_after_g_prefix() {
        assert_eq!(feed_all("g"), ParseResult::Pending);
    }

    #[test]
    fn pending_after_quote() {
        assert_eq!(feed_all("\""), ParseResult::Pending);
    }

    #[test]
    fn pending_after_count() {
        assert_eq!(feed_all("3"), ParseResult::Pending);
    }

    #[test]
    fn pending_after_register_name() {
        assert_eq!(feed_all("\"a"), ParseResult::Pending);
    }

    #[test]
    fn pending_after_operator_and_count() {
        assert_eq!(feed_all("d3"), ParseResult::Pending);
    }

    // ── Invalid sequences ────────────────────────────────────────────────────

    #[test]
    fn invalid_d_q() {
        assert_eq!(feed_all("dq"), ParseResult::Invalid);
    }

    #[test]
    fn invalid_z() {
        assert_eq!(feed_all("z"), ParseResult::Invalid);
    }

    #[test]
    fn invalid_unknown_register() {
        assert_eq!(feed_all("\"1"), ParseResult::Invalid);
    }

    #[test]
    fn invalid_g_other() {
        assert_eq!(feed_all("gz"), ParseResult::Invalid);
    }

    // ── Parser resets after complete ─────────────────────────────────────────

    #[test]
    fn parser_resets_after_complete() {
        let mut parser = ActionParser::new();
        let r1 = parser.feed('w');
        assert!(matches!(r1, ParseResult::Complete(_)));
        assert!(!parser.is_pending());
        assert_eq!(parser.pending(), "");
        // Next sequence starts fresh.
        let r2 = parser.feed('b');
        assert!(matches!(r2, ParseResult::Complete(_)));
    }

    #[test]
    fn parser_resets_after_invalid() {
        let mut parser = ActionParser::new();
        let r1 = parser.feed('z');
        assert_eq!(r1, ParseResult::Invalid);
        assert!(!parser.is_pending());
        // Next sequence starts fresh.
        let r2 = parser.feed('w');
        assert!(matches!(r2, ParseResult::Complete(_)));
    }

    // ── Text objects (aw / iw) ───────────────────────────────────────────────

    #[test]
    fn change_around_word_caw() {
        assert_eq!(
            feed_all("caw"),
            complete(1, None, Operator::Change, Motion::AWord)
        );
    }

    #[test]
    fn delete_around_word_daw() {
        assert_eq!(
            feed_all("daw"),
            complete(1, None, Operator::Delete, Motion::AWord)
        );
    }

    #[test]
    fn yank_around_word_yaw() {
        assert_eq!(
            feed_all("yaw"),
            complete(1, None, Operator::Yank, Motion::AWord)
        );
    }

    #[test]
    fn change_inner_word_ciw() {
        assert_eq!(
            feed_all("ciw"),
            complete(1, None, Operator::Change, Motion::IWord)
        );
    }

    #[test]
    fn delete_inner_word_diw() {
        assert_eq!(
            feed_all("diw"),
            complete(1, None, Operator::Delete, Motion::IWord)
        );
    }

    #[test]
    fn register_a_caw() {
        assert_eq!(
            feed_all("\"acaw"),
            complete(1, Some('a'), Operator::Change, Motion::AWord)
        );
    }

    #[test]
    fn register_a_ciw() {
        assert_eq!(
            feed_all("\"aciw"),
            complete(1, Some('a'), Operator::Change, Motion::IWord)
        );
    }

    #[test]
    fn text_object_unknown_cax_is_invalid() {
        assert_eq!(feed_all("cax"), ParseResult::Invalid);
    }

    #[test]
    fn text_object_unknown_cix_is_invalid() {
        assert_eq!(feed_all("cix"), ParseResult::Invalid);
    }

    #[test]
    fn pending_after_ca() {
        assert_eq!(feed_all("ca"), ParseResult::Pending);
    }

    #[test]
    fn pending_after_ci() {
        assert_eq!(feed_all("ci"), ParseResult::Pending);
    }

    // ── Inclusivity flag ────────────────────────────────────────────────────

    #[test]
    fn word_end_is_inclusive() {
        assert!(Motion::WordEnd.is_inclusive());
    }

    #[test]
    fn line_end_is_inclusive() {
        assert!(Motion::LineEnd.is_inclusive());
    }

    #[test]
    fn aword_is_inclusive() {
        assert!(Motion::AWord.is_inclusive());
    }

    #[test]
    fn iword_is_not_inclusive() {
        assert!(!Motion::IWord.is_inclusive());
    }

    #[test]
    fn word_forward_is_exclusive() {
        assert!(!Motion::WordForward.is_inclusive());
    }

    // ── Navigation flag ──────────────────────────────────────────────────────

    #[test]
    fn down_is_navigation() {
        assert!(Motion::Down.is_navigation());
    }

    #[test]
    fn word_forward_is_not_navigation() {
        assert!(!Motion::WordForward.is_navigation());
    }
}
