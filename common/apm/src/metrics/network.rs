use super::{
    auto_flush_from, exponential_buckets, make_auto_flush_static_metric, register_histogram_vec,
    register_int_counter_vec, HistogramVec, IntCounterVec,
};

use lazy_static::lazy_static;

make_auto_flush_static_metric! {
    pub label_enum MessageDirection {
        sent,
        received,
    }

    pub label_enum ProtocolKind {
        rpc,
    }

    pub label_enum RPCResult {
        success,
        timeout,
    }

    pub struct MessageCounterVec: LocalIntCounter {
        "direction" => MessageDirection,
    }

    pub struct RPCResultCounterVec: LocalIntCounter {
        "result" => RPCResult,
    }

    pub struct ProtocolTimeHistogramVec: LocalHistogram {
        "type" => ProtocolKind,
    }
}

lazy_static! {
    pub static ref NETWORK_MESSAGE_COUNT_VEC: IntCounterVec = register_int_counter_vec!(
        "muta_network_message_total",
        "Total number of network message",
        &["direction"]
    )
    .expect("network message total");
    pub static ref NETWORK_RPC_RESULT_COUNT_VEC: IntCounterVec = register_int_counter_vec!(
        "muta_network_rpc_result_total",
        "Total number of network rpc result",
        &["result"]
    )
    .expect("network rpc result total");
    pub static ref NETWORK_PROTOCOL_TIME_HISTOGRAM_VEC: HistogramVec = register_histogram_vec!(
        "muta_network_protocol_time_cost_seconds",
        "Network protocol time cost",
        &["type"],
        exponential_buckets(1.0, 2.0, 10).expect("network protocol time expontial")
    )
    .expect("network protocol time cost");
}

lazy_static! {
    pub static ref NETWORK_MESSAGE_COUNT_VEC_STATIC: MessageCounterVec =
        auto_flush_from!(NETWORK_MESSAGE_COUNT_VEC, MessageCounterVec);
    pub static ref NETWORK_RPC_RESULT_COUNT_VEC_STATIC: RPCResultCounterVec =
        auto_flush_from!(NETWORK_RPC_RESULT_COUNT_VEC, RPCResultCounterVec);
    pub static ref NETWORK_PROTOCOL_TIME_HISTOGRAM_VEC_STATIC: ProtocolTimeHistogramVec = auto_flush_from!(
        NETWORK_PROTOCOL_TIME_HISTOGRAM_VEC,
        ProtocolTimeHistogramVec
    );
}
