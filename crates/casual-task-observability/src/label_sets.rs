/// A label that would have widened a metric past what it declared.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CardinalityError {
    /// The metric did not declare this label key.
    #[error(
        "metric `{metric}` does not declare label `{key}` (declared: {declared}); \
         see docs/46 §Cardinality discipline"
    )]
    UndeclaredLabel {
        /// The metric the label was attached to.
        metric: &'static str,
        /// The offending key.
        key: &'static str,
        /// The keys the metric does declare, comma separated.
        declared: String,
    },
    /// A cardinality-bearing value was attached to a key it was not minted for.
    #[error(
        "label `{key}` on metric `{metric}` was given a value minted for \
         `{minted_for}`. That value's cardinality is a documented trade for \
         `{minted_for}` only; on `{key}` it is an unbounded series \
         (docs/46 §Cardinality discipline)"
    )]
    MisplacedValue {
        /// The metric the label was attached to.
        metric: &'static str,
        /// The key it was attached to.
        key: &'static str,
        /// The key the value may be used with.
        minted_for: &'static str,
    },
    /// The metric declared more labels than [`MAX_LABELS_PER_METRIC`].
    #[error("metric `{metric}` declares {declared} labels; the cap is {max}")]
    TooManyLabels {
        /// The offending metric.
        metric: &'static str,
        /// How many it declared.
        declared: usize,
        /// The cap.
        max: usize,
    },
    /// [`InvestigationAllowList`] is full.
    #[error(
        "investigation allow-list is full ({max} workspaces); \
         revoke one before admitting another (docs/46 §Cardinality discipline)"
    )]
    AllowListFull {
        /// The cap.
        max: usize,
    },
}

/// The labels attached to one observation of one metric.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LabelSet {
    metric: Metric,
    pairs: BTreeMap<LabelKey, LabelValue>,
}

impl LabelSet {
    /// Begin a label set for `metric`.
    pub fn for_metric(metric: Metric) -> Self {
        Self {
            metric,
            pairs: BTreeMap::new(),
        }
    }

    /// Attach a label while enforcing the metric's declaration.
    pub fn with(
        mut self,
        key: LabelKey,
        value: impl Into<LabelValue>,
    ) -> Result<Self, CardinalityError> {
        let declared = self.metric.labels();
        if declared.len() > MAX_LABELS_PER_METRIC {
            return Err(CardinalityError::TooManyLabels {
                metric: self.metric.name().as_str(),
                declared: declared.len(),
                max: MAX_LABELS_PER_METRIC,
            });
        }
        if !declared.contains(&key) {
            return Err(CardinalityError::UndeclaredLabel {
                metric: self.metric.name().as_str(),
                key: key.as_str(),
                declared: declared
                    .iter()
                    .map(LabelKey::as_str)
                    .collect::<Vec<_>>()
                    .join(", "),
            });
        }
        let value = value.into();
        if let Some(minted_for) = value.minted_for()
            && minted_for != key
        {
            return Err(CardinalityError::MisplacedValue {
                metric: self.metric.name().as_str(),
                key: key.as_str(),
                minted_for: minted_for.as_str(),
            });
        }
        self.pairs.insert(key, value);
        Ok(self)
    }

    /// The metric these labels belong to.
    pub fn metric(&self) -> Metric {
        self.metric
    }

    /// The labels, in deterministic key order.
    pub fn pairs(&self) -> Vec<(&'static str, &str)> {
        self.pairs
            .iter()
            .map(|(key, value)| (key.as_str(), value.as_str()))
            .collect()
    }
}
