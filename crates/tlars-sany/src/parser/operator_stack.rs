//! `OperatorStack` + `Operators`: precedence *ranges* `(low, high)`; overlapping ranges with
//! neither `succ` nor `prec` holding is "Precedence conflict between ops ...".
//! Left-associativity only for the same operator; `\X` collected n-ary unless parenthesised;
//! prefix `-` resolved on the stack (`-.`); `.` record access via `reduceRecord`.

pub(crate) struct Operator {
    pub(crate) id: crate::Atom,
    pub(crate) low: u8,
    pub(crate) high: u8,
    pub(crate) assoc: Assoc,
    pub(crate) fix: Fixness,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Assoc {
    None,
    Left,
    Right,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Fixness {
    Prefix,
    Infix,
    Postfix,
    Nfix,
}

#[derive(Default)]
pub(crate) struct OperatorStack {
    frames: Vec<Vec<super::SyntaxTreeNode>>,
}

impl OperatorStack {
    pub(crate) fn push_on_stack(&mut self, node: super::SyntaxTreeNode, op: Option<&Operator>) -> Result<(), super::ParseError> {
        todo!()
    }
    pub(crate) fn reduce_stack(&mut self) -> Result<super::SyntaxTreeNode, super::ParseError> {
        todo!()
    }
}
