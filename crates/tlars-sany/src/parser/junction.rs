//! `JunctionListContext`: `/\` and `\/` bullet lists. A new item needs the same column and
//! kind; every token inside an item must be strictly right of the bullet's column
//! (`isAboveCurrent`, a semantic lookahead on every expression continuation).

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum BulletKind {
    And,
    Or,
}

#[derive(Default)]
pub(crate) struct JunctionListContext {
    stack: Vec<(BulletKind, u32)>,
}

impl JunctionListContext {
    pub(crate) fn start_new_junction_list(&mut self, column: u32, kind: BulletKind) {
        todo!()
    }
    pub(crate) fn is_new_bullet(&self, column: u32, kind: BulletKind) -> bool {
        todo!()
    }
    pub(crate) fn is_above_current(&self, column: u32) -> bool {
        todo!()
    }
    pub(crate) fn terminate_current_junction_list(&mut self) {
        todo!()
    }
}
