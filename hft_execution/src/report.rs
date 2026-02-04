use std::fs;

use crate::performance::InstrumentReport;

pub fn write_markdown_report(path: &str, reports: &[InstrumentReport]) -> std::io::Result<()> {
    let mut output = String::new();
    output.push_str("# Performance Report\n\n");
    output.push_str("## Summary\n\n");

    for report in reports {
        output.push_str(&format!(
            "### {}\n\n",
            report.instrument.symbol
        ));
        output.push_str(&format!(
            "- Instrument Type: {:?}\n",
            report.instrument.instrument_type
        ));
        output.push_str(&format!("- Side: {:?}\n", report.side));
        output.push_str(&format!(
            "- Decision Price: {:.4}\n",
            report.decision_price
        ));
        output.push_str(&format!(
            "- Avg Execution Price: {:.4}\n",
            report.avg_execution_price
        ));
        output.push_str(&format!(
            "- Implementation Shortfall (bps): {:.2}\n",
            report.implementation_shortfall_bps
        ));
        output.push_str(&format!(
            "- VWAP Comparison (bps): {:.2}\n",
            report.vwap_bps
        ));
        output.push_str(&format!(
            "- Fill Rate: {:.2}%\n",
            report.fill_rate * 100.0
        ));
        output.push_str(&format!(
            "- Maker Ratio: {:.2}\n",
            report.maker_ratio
        ));
        output.push_str(&format!(
            "- Taker Ratio: {:.2}\n",
            report.taker_ratio
        ));
        output.push_str(&format!(
            "- Adverse Selection 1s (bps): {:.2}\n",
            report.adverse_1s_bps
        ));
        output.push_str(&format!(
            "- Adverse Selection 5s (bps): {:.2}\n",
            report.adverse_5s_bps
        ));
        output.push_str(&format!(
            "- Adverse Selection 30s (bps): {:.2}\n\n",
            report.adverse_30s_bps
        ));
    }

    fs::write(path, output)
}
