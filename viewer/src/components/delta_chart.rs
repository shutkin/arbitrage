use leptos::prelude::*;
use leptos::{component, view, IntoView};
use leptos_chartistry::{
    AspectRatio, AxisMarker, Chart, IntoInner, Legend, Line, Series, TickLabels, Tooltip,
    XGridLine, XGuideLine, YGridLine, YGuideLine,
};
use crate::client_model::TimedValues;

#[component]
pub fn DeltaChart(data: Signal<Vec<TimedValues>>) -> impl IntoView {
    let series = Series::new(|data: &TimedValues| data.timestamp)
        .line(Line::new(move |data: &TimedValues| data.values[0]).with_name("Sell"))
        .line(Line::new(move |data: &TimedValues| data.values[1]).with_name("Buy"));
    view! {
        class = "chart",
            <Chart
                aspect_ratio=AspectRatio::from_env_width_apply_ratio(3.0)
                series=series
                data=data

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
