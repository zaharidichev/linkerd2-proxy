impl<T, N, L> super::Layer<T, T, N> for Option<L>
where
    N: super::Stack<T>,
    L: super::Layer<T, T, N, Value = N::Value, Error = N::Error>,
{
    type Value = <super::Either<N, L::Stack> as super::Stack<T>>::Value;
    type Error = <super::Either<N, L::Stack> as super::Stack<T>>::Error;
    type Stack = super::Either<N, L::Stack>;

    fn bind(&self, next: N) -> Self::Stack {
        match self.as_ref() {
            None => super::Either::A(next),
            Some(ref m) => super::Either::B(m.bind(next)),
        }
    }
}
