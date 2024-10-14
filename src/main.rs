use std::collections::LinkedList;
use std::sync::Arc;
use alfred_rs::connection::{Receiver, Sender};
use alfred_rs::error::Error;
use alfred_rs::interface_module::InterfaceModule;
use alfred_rs::log::{debug, error, warn};
use alfred_rs::message::MessageType;
use alfred_rs::pubsub_connection::MODULE_INFO_TOPIC_REQUEST;
use teloxide::Bot;
use teloxide::prelude::{Message, Requester};
use teloxide::types::{ChatId, InputFile};
use tokio::sync::Mutex;
use teloxide::net::Download;
use tokio::fs;

const MODULE_NAME: &'static str = "telegram";
const RESPONSE_TOPIC: &'static str = "telegram";

#[tokio::main(flavor = "multi_thread", worker_threads = 2)]
async fn main() -> Result<(), Error> {
    env_logger::init();
    debug!("Starting telegram module...");
    let module = InterfaceModule::new(MODULE_NAME).await?;

    let bot_token = module.config.get_module_value("bot_token").expect("BOT token not found");
    let callback_topic = module.config.get_module_value("callback");
    if callback_topic.is_none() {
        warn!("Unknown callback topic.")
    }
    let bot = Bot::new(bot_token);
    let bot2 = bot.clone();
    let alfred_subscriber = Arc::new(Mutex::new(module.connection.subscriber));
    let alfred_publisher1 = Arc::new(Mutex::new(module.connection.publisher));
    let alfred_publisher2 = alfred_publisher1.clone();

    tokio::spawn(async move {
        let alfred_subscriber = alfred_subscriber.clone();
        let alfred_publisher = alfred_publisher1.clone();
        let bot2 = bot2.clone();
        async move {
            debug!("Configuring Alfred receiver...");
            alfred_subscriber.lock().await.listen(RESPONSE_TOPIC).await.expect("Error on alfred subscription");
            debug!("Configured Alfred receiver!!!");
            loop {
                debug!("Waiting for new Alfred messages...");
                // TODO: remove all ".expect" and substitute with other
                let mut subscriber = alfred_subscriber.lock().await;
                let (topic, message) = subscriber.receive().await.expect("Error on receiving Alfred Message");
                let mut publisher = alfred_publisher.lock().await;
                // TODO: add macro for manage_module_info_request
                if topic == MODULE_INFO_TOPIC_REQUEST {
                    publisher.send_module_info(MODULE_NAME).await.expect("Error replying to MODULE_INFO_TOPIC_REQUEST");
                    continue;
                }
                debug!("New message on topic {}: {:?}", topic, message);
                match topic.as_str() {
                    RESPONSE_TOPIC => {
                        let chat_id = ChatId(message.sender.parse().unwrap());
                        match message.message_type {
                            MessageType::TEXT => {
                                bot2.send_message(chat_id, message.text).await.expect("Error on send message to telegram");
                            }
                            MessageType::AUDIO => {
                                let input_file = InputFile::file(message.text);
                                bot2.send_voice(chat_id, input_file).await.expect("Error on send voice to telegram");
                            }
                            _ => {
                                warn!("Unsupported MessageType");
                            }
                        }
                    }
                    _ => {
                        warn!("Unmanaged topic {}", topic);
                    }
                };
            }
        }.await;
    });


    debug!("Configuring telegram receiver...");
    teloxide::repl(bot.clone(), move |bot: Bot, msg: Message| {
        let alfred_publisher = alfred_publisher2.clone();
        let callback_topic = callback_topic.clone();
        async move {
            let alfred_msg_res = telegram_msg_to_alfred_msg(msg, &bot).await;
            if alfred_msg_res.is_err() {
                error!("{}", alfred_msg_res.err().unwrap());
                return Ok(());
            }
            let alfred_msg = alfred_msg_res.unwrap();
            if callback_topic.is_some() {
                alfred_publisher.lock().await.send(callback_topic.unwrap().as_str(), &alfred_msg).await.expect("Error on publish");
            }
            Ok(())
        }
    }).await;
    debug!("Configured telegram receiver!!!");
    Ok(())
}

async fn telegram_msg_to_alfred_msg(msg: Message, bot: &Bot) -> Result<alfred_rs::message::Message, String> {
    // TODO: implement other types of message
    // TODO: add other info to params property
    let mut message_type = MessageType::TEXT;
    let mut text: String = msg.text().unwrap_or("").to_string();
    if msg.voice().is_some() {
        let voice_file_id = msg.voice().ok_or("err")?.clone().file.id;
        let file = bot.get_file(voice_file_id.clone()).await.map_err(|e| e.to_string())?;
        let current_dir = std::env::current_dir().map_err(|e| e.to_string())?.display().to_string();
        let dst_filename = format!("{current_dir}/tmp/{voice_file_id}.ogg");
        let mut dst = fs::File::create(dst_filename.clone()).await.map_err(|_| "err3".to_string())?;
        bot.download_file(&file.path, &mut dst).await.map_err(|_| "err4".to_string())?;
        text = dst_filename.to_string();
        message_type = MessageType::AUDIO;
    }
    debug!("Received {:?} message {} from {}", message_type, text, msg.chat.id);
    Ok(new_callback_msg(text, msg.chat.id.to_string(), message_type))
}

fn new_callback_msg(text: String, sender: String, message_type: MessageType) -> alfred_rs::message::Message {
    alfred_rs::message::Message {
        text,
        starting_module: MODULE_NAME.to_string(),
        // TODO: remove request_topic?
        request_topic: "".to_string(),
        response_topics: LinkedList::from([RESPONSE_TOPIC.to_string()]),
        sender,
        message_type,
        params: Default::default(),
    }
}
