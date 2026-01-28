use flipperzero::notification::{NotificationSequence, messages};
use flipperzero::notification_sequence;

pub const MANUAL_POWER_OFF: NotificationSequence =
    notification_sequence![messages::RED_255, messages::DELAY_100,];
pub const MANUAL_POWER_ON: NotificationSequence =
    notification_sequence![messages::RED_255, messages::BLUE_255, messages::DELAY_100,];
pub const DAYTIME_CHANGE: NotificationSequence =
    notification_sequence![messages::RED_255, messages::GREEN_255, messages::DELAY_1000,];
