use ndarray::ArrayD;

#[derive(Clone, Debug, Default)]
pub struct ArrayStatistics {
    pub element_count: usize,
    pub finite_count: usize,
    pub nan_count: usize,
    pub min: Option<f64>,
    pub max: Option<f64>,
    pub mean: Option<f64>,
    pub std_dev: Option<f64>,
}

#[derive(Clone, Debug, Default)]
pub struct ArrayPreview {
    pub column_labels: Vec<String>,
    pub rows: Vec<Vec<String>>,
}

pub fn compute_statistics(values: &ArrayD<f64>) -> ArrayStatistics {
    let element_count = values.len();
    let mut finite = Vec::new();

    for &v in values.iter() {
        if v.is_finite() {
            finite.push(v);
        }
    }

    let finite_count = finite.len();
    let nan_count = element_count - finite_count;

    if finite.is_empty() {
        return ArrayStatistics {
            element_count,
            finite_count,
            nan_count,
            ..Default::default()
        };
    }

    let min = finite.iter().copied().reduce(f64::min);
    let max = finite.iter().copied().reduce(f64::max);
    let mean = Some(finite.iter().sum::<f64>() / finite_count as f64);

    let std_dev = mean.map(|m| {
        let var = finite.iter().map(|v| (v - m).powi(2)).sum::<f64>() / finite_count as f64;
        var.sqrt()
    });

    ArrayStatistics {
        element_count,
        finite_count,
        nan_count,
        min,
        max,
        mean,
        std_dev,
    }
}

pub fn build_preview(values: &ArrayD<f64>, max_rows: usize, max_cols: usize) -> ArrayPreview {
    let shape = values.shape();

    if shape.is_empty() || (shape.len() == 1 && shape[0] <= 1) {
        let text = values
            .iter()
            .next()
            .map(|v| format_cell(*v))
            .unwrap_or_else(|| "—".to_string());
        return ArrayPreview {
            column_labels: vec!["value".to_string()],
            rows: vec![vec![text]],
        };
    }

    if shape.len() == 1 {
        let n = shape[0].min(max_cols);
        let column_labels: Vec<String> = (0..n).map(|i| i.to_string()).collect();
        let row = (0..n)
            .map(|i| format_cell(values[[i]]))
            .collect::<Vec<_>>();
        return ArrayPreview {
            column_labels,
            rows: vec![row],
        };
    }

    if shape.len() >= 2 {
        let height = shape[shape.len() - 2].min(max_rows);
        let width = shape[shape.len() - 1].min(max_cols);
        let column_labels: Vec<String> = (0..width).map(|i| i.to_string()).collect();
        let mut rows = Vec::with_capacity(height);

        for row in 0..height {
            let mut line = Vec::with_capacity(width);
            for col in 0..width {
                let mut index = vec![0usize; shape.len()];
                if shape.len() > 2 {
                    for d in 0..shape.len() - 2 {
                        index[d] = 0;
                    }
                }
                index[shape.len() - 2] = row;
                index[shape.len() - 1] = col;
                line.push(format_cell(values[index.as_slice()]));
            }
            rows.push(line);
        }

        return ArrayPreview {
            column_labels,
            rows,
        };
    }

    ArrayPreview::default()
}

pub fn format_statistics(stats: &ArrayStatistics) -> String {
    let mut lines = vec!["Statistics (loaded subset):".to_string()];
    lines.push(format!("    elements: {}", stats.element_count));
    lines.push(format!("    finite:   {}", stats.finite_count));
    if stats.nan_count > 0 {
        lines.push(format!("    NaN:      {}", stats.nan_count));
    }
    if let Some(v) = stats.min {
        lines.push(format!("    min:      {v:.6}"));
    }
    if let Some(v) = stats.max {
        lines.push(format!("    max:      {v:.6}"));
    }
    if let Some(v) = stats.mean {
        lines.push(format!("    mean:     {v:.6}"));
    }
    if let Some(v) = stats.std_dev {
        lines.push(format!("    std:      {v:.6}"));
    }
    lines.join("\n")
}

pub fn format_preview_table(preview: &ArrayPreview) -> String {
    if preview.rows.is_empty() {
        return "Preview: (empty)".to_string();
    }

    let mut lines = vec!["Data preview (subset):".to_string()];
    lines.push(format!("    {}", preview.column_labels.join("\t")));
    for row in &preview.rows {
        lines.push(format!("    {}", row.join("\t")));
    }
    if preview.rows.len() >= 8 {
        lines.push("    …".to_string());
    }
    lines.join("\n")
}

fn format_cell(v: f64) -> String {
    if !v.is_finite() {
        return "NaN".to_string();
    }
    if v.fract() == 0.0 && v.abs() < 1e12 {
        format!("{v:.0}")
    } else {
        format!("{v:.4}")
    }
}
