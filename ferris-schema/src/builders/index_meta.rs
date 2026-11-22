use crate::registry::IndexMeta;

impl IndexMeta {
    /// A new secondary index over `fields`.
    #[must_use]
    pub fn new(
        name: impl Into<String>,
        fields: impl IntoIterator<Item = impl Into<String>>,
    ) -> Self {
        Self {
            name: name.into(),
            fields: fields.into_iter().map(Into::into).collect(),
            unique: false,
            ..Self::default()
        }
    }

    /// Make the index unique.
    #[must_use]
    pub fn unique(mut self) -> Self {
        self.unique = true;
        self
    }
}
