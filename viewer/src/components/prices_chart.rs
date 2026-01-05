use std::collections::HashMap;
use leptos::{component, view, IntoView};
use leptos::prelude::*;
use leptos_chartistry::{AspectRatio, AxisMarker, Chart, IntoInner, Legend, Line, Series, TickLabels, Tooltip, XGridLine, XGuideLine, YGridLine, YGuideLine};
use crate::client_model::{InstrumentOption, TimedValues};

fn map_data(data: Signal<Vec<TimedValues>>, names: Signal<Vec<String>>, instruments: Signal<Vec<InstrumentOption>>) -> Signal<Vec<TimedValues>> {
    Signal::derive(move || {
        let data = data.get();
        let names = names.get();
        let instruments = instruments.get();

        let mut map = HashMap::new();
        for (inst_index, inst) in instruments.iter().enumerate() {
            if let Some((name_index, _)) = names.iter().enumerate().find(|(_, name)| *name == &inst.label) {
                map.insert(name_index, inst_index);
            }
        }
        data.iter().map(|item| {
            let mut values = vec![f64::NAN; instruments.len()];
            item.values.iter().enumerate().for_each(|(data_index, value)| {
                let inst_index = map.get(&data_index).cloned().unwrap_or_default();
                values[inst_index] = *value;
            });
            TimedValues {
                timestamp: item.timestamp,
                values,
            }
        }).collect::<Vec<_>>()
    })
}

#[component]
pub fn PricesChart(data: Signal<Vec<TimedValues>>, names: Signal<Vec<String>>, instruments: Signal<Vec<InstrumentOption>>) -> impl IntoView {
    let mut series = Series::new(|data: &TimedValues| data.timestamp);
    for (i, inst) in instruments.get().iter().enumerate() {
        series = series.line(Line::new(move |data: &TimedValues| data.values[i]).with_name(inst.label.clone()));
    }
    let data_mapped = map_data(data, names, instruments);
    view! {
        class = "chart",
            <Chart
                aspect_ratio=AspectRatio::from_env_width_apply_ratio(3.0)
                series=series
                data=data_mapped

                left=TickLabels::aligned_floats()
                bottom=Legend::end()
                inner=[
                    // Standard set of inner layout options
                    AxisMarker::left_edge().into_inner(),
                    AxisMarker::bottom_edge().into_inner(),
                    XGridLine::default().into_inner(),
                    YGridLine::default().into_inner(),
                    YGuideLine::over_mouse().into_inner(),
                    XGuideLine::over_data().into_inner(),
                ]
                tooltip=Tooltip::left_cursor().show_x_ticks(true)
            />
    }
}
