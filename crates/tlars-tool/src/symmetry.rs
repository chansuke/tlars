//! `MVPerm` / `MVPerms`: SYMMETRY as a closed permutation group over model-value indices.

pub(crate) struct MvPerm {
    /// `(from index, to index)` pairs, model values outside the symmetry set map to themselves.
    pub(crate) pairs: Box<[(u16, u16)]>,
}

pub(crate) fn permutation_group(generators: &[MvPerm]) -> Vec<MvPerm> {
    todo!("MVPerms.permutationSubgroup closure")
}
