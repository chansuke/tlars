//! `MVPerm` / `MVPerms`: SYMMETRY as a closed permutation group over model-value indices.

use crate::error::EvalResult;
use crate::eval::Ev;
use crate::value::{ModelValue, Value};

/// `MVPerm` (`MVPerm.java`): one slot per model value in creation order, holding the model
/// value it maps to. A slot stays empty where the permutation fixes the value, so `size()`
/// counts the moved ones and a permutation of size 0 is dropped.
pub(crate) struct MvPerm {
    elems: Box<[Option<ModelValue>]>,
    count: usize,
}

/// `ModelValue.equals`, which `MVPerm`'s `put` and `equals` compare with: two
/// differently-typed model values are an error, not an answer.
fn same(ev: &mut Ev<'_>, a: ModelValue, b: ModelValue) -> EvalResult<bool> {
    Value::Model(a).tlc_equals(ev, &Value::Model(b))
}

impl MvPerm {
    fn empty(model_values: usize) -> MvPerm {
        MvPerm { elems: vec![None; model_values].into_boxed_slice(), count: 0 }
    }

    /// `MVPerm.get(k)`: the image of `k`, or `None` where the permutation fixes it (TLC's
    /// `null`, which `ModelValue.permute` turns back into the value itself).
    ///
    /// `Err` is the `ArrayIndexOutOfBoundsException` of a model value created after the
    /// permutations were sized, which is what `TLCExt!TLCModelValue` does outside constant
    /// processing. `ModelValue.add`'s own comment calls that bogus behaviour.
    pub(crate) fn get(&self, k: ModelValue) -> EvalResult<Option<ModelValue>> {
        match self.elems.get(k.index as usize) {
            Some(slot) => Ok(*slot),
            None => Err(crate::error::EvalError::index_out_of_bounds(k.index as usize, self.elems.len())),
        }
    }

    pub(crate) fn size(&self) -> usize {
        self.count
    }

    /// `MVPerm.put(k, elem)`: the first mapping for a slot wins, and a value mapped to
    /// itself is not recorded.
    fn put(&mut self, ev: &mut Ev<'_>, k: ModelValue, elem: ModelValue) -> EvalResult<()> {
        if !same(ev, k, elem)? && self.elems[k.index as usize].is_none() {
            self.elems[k.index as usize] = Some(elem);
            self.count += 1;
        }
        Ok(())
    }

    /// The private `put(int, ModelValue)` `compose` uses.
    fn put_at(&mut self, i: usize, elem: Option<ModelValue>) {
        if self.elems[i].is_none()
            && let Some(elem) = elem
        {
            self.elems[i] = Some(elem);
            self.count += 1;
        }
    }

    /// `MVPerm.compose(perm)` (`MVPerm.java:78-96`).
    fn compose(&self, ev: &mut Ev<'_>, other: &MvPerm, model_values: &[ModelValue]) -> EvalResult<MvPerm> {
        let mut res = MvPerm::empty(self.elems.len());
        for (i, slot) in self.elems.iter().enumerate() {
            match *slot {
                None => res.put_at(i, other.elems[i]),
                Some(mv) => match other.elems[mv.index as usize] {
                    None => res.put_at(i, Some(mv)),
                    Some(mv1) if !same(ev, model_values[i], mv1)? => res.put_at(i, Some(mv1)),
                    Some(_) => {}
                },
            }
        }
        Ok(res)
    }

    /// `MVPerm.equals`, which the `util.Set` of the group de-duplicates with.
    fn same_as(&self, ev: &mut Ev<'_>, other: &MvPerm) -> EvalResult<bool> {
        for (a, b) in self.elems.iter().zip(other.elems.iter()) {
            let equal = match (a, b) {
                (None, None) => true,
                (Some(x), Some(y)) => same(ev, *x, *y)?,
                _ => false,
            };
            if !equal {
                return Ok(false);
            }
        }
        Ok(true)
    }

    /// `MVPerm.getAllModelValues()`: the images this permutation holds, in slot order.
    pub(crate) fn images(&self) -> impl Iterator<Item = ModelValue> + use<'_> {
        self.elems.iter().filter_map(|v| *v)
    }
}

/// One generator of the group: the `(domain, range)` pairs of a symmetry function, in the
/// order `MVPerms.permutationSubgroup` reads them off the function record.
pub(crate) struct Generator {
    pub(crate) pairs: Vec<(ModelValue, ModelValue)>,
}

/// `MVPerms.permutationSubgroup` (`MVPerms.java:19-70`): the generators, de-duplicated and
/// with the identity dropped, then closed under composition. The result is in `permVec`
/// order, which is the generators as they were met followed by the compositions as they
/// were found.
pub(crate) fn permutation_group(ev: &mut Ev<'_>, generators: &[Generator], model_values: &[ModelValue]) -> EvalResult<Vec<MvPerm>> {
    let mut perms: Vec<MvPerm> = Vec::new();
    for g in generators {
        let mut perm = MvPerm::empty(model_values.len());
        for &(dval, rval) in &g.pairs {
            perm.put(ev, dval, rval)?;
        }
        if perm.size() > 0 && !contains(ev, &perms, &perm)? {
            perms.push(perm);
        }
    }
    // Close the group: compose every generator with everything found since the last round.
    let gsz = perms.len();
    let mut sz0 = 0;
    loop {
        let sz1 = perms.len();
        for i in 0..gsz {
            for j in sz0..sz1 {
                let perm = perms[i].compose(ev, &perms[j], model_values)?;
                if perm.size() > 0 && !contains(ev, &perms, &perm)? {
                    perms.push(perm);
                }
            }
        }
        if sz1 == perms.len() {
            break;
        }
        sz0 = sz1;
    }
    Ok(perms)
}

fn contains(ev: &mut Ev<'_>, perms: &[MvPerm], perm: &MvPerm) -> EvalResult<bool> {
    for p in perms {
        if p.same_as(ev, perm)? {
            return Ok(true);
        }
    }
    Ok(false)
}
