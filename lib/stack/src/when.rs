/// Produces `Stack`s that include an `L`-typed layer only when a `P`-typed
/// `Predicate` evaluates to `true` for the stack's target.
#[derive(Clone)]
pub struct Layer<P, L> {
    predicate: P,
    inner: L,
}

/// A condition that determines whether a layer should be applied for a `T`-typed
/// target.
pub trait Predicate<T> {
    /// Returns true iff a conditional layer should be applied for this target.
    fn affirm(&self, target: &T) -> bool;
}

/// When the `P`-typed predicate applies to a target, the `L`
/// `Predicate` evaluates to `true` for the stack's target.
pub struct Stack<P, N, L> {
    predicate: P,
    next: N,
    layer: L,
}

// === impl Layer ===

impl<P, L> Layer<P, L> {
    pub fn new<T, N>(predicate: P, inner: L) -> Self
    where
        P: Predicate<T> + Clone,
        N: super::Stack<T> + Clone,
        L: super::Layer<T, T, N, Error = N::Error> + Clone,
        L::Stack: super::Stack<T>,
    {
        Self  {
            predicate,
            inner,
        }
    }
}

impl<T, P, N, L> super::Layer<T, T, N> for Layer<P, L>
where
    P: Predicate<T> + Clone,
    N: super::Stack<T> + Clone,
    L: super::Layer<T, T, N, Error = N::Error> + Clone,
    L::Stack: super::Stack<T>,
{
    type Value = <Stack<P, N, L> as super::Stack<T>>::Value;
    type Error = <Stack<P, N, L> as super::Stack<T>>::Error;
    type Stack = Stack<P, N, L>;

    fn bind(&self, next: N) -> Self::Stack {
        Stack {
            predicate: self.predicate.clone(),
            next,
            layer: self.inner.clone(),
        }
    }
}

// === impl Stack ===

impl<P, N, L> Clone for Stack<P, N, L>
where
    P: Clone,
    N: Clone,
    L: Clone,
{
    fn clone(&self) -> Self {
        Self {
            predicate: self.predicate.clone(),
            next: self.next.clone(),
            layer: self.layer.clone(),
        }
    }
}

impl<T, P, N, L> super::Stack<T> for Stack<P, N, L>
where
    P: Predicate<T> + Clone,
    N: super::Stack<T> + Clone,
    L: super::Layer<T, T, N, Error = N::Error>,
    L::Stack: super::Stack<T>,
{
    type Value = super::Either<N::Value, L::Value>;
    type Error = N::Error;

    fn make(&self, target: &T) -> Result<Self::Value, Self::Error> {
        if !self.predicate.affirm(&target) {
            self.next
                .make(&target)
                .map(super::Either::A)
        } else {
            self.layer
                .bind(self.next.clone())
                .make(&target)
                .map(super::Either::B)
        }
    }
}

// === impl Predicate<T> for Fn(&T) -> bool ===

impl<T, F: Fn(&T) -> bool> Predicate<T> for F {
    fn affirm(&self, t: &T) -> bool {
        (self)(t)
    }
}
