#![crate_name="theca"]
#![crate_type="lib"]
#![allow(unstable)]

extern crate core;
extern crate libc;
extern crate time;
extern crate docopt;
extern crate "rustc-serialize" as rustc_serialize;
extern crate regex;
extern crate crypto;
extern crate term;

// std lib imports
use std::os::{getenv};
use std::io::fs::{PathExtensions, mkdir};
use std::io::{File, Truncate, Write, Read, Open,
              stdin, USER_RWX};
use std::iter::{repeat};

// random things
use regex::{Regex};
use rustc_serialize::{Encodable, Encoder, json};
use time::{now, strftime};

// crypto imports
use lineformat::{LineFormat};
use utils::c::{istty};
use utils::{drop_to_editor, pretty_line, format_field,
            get_yn_input, sorted_print, localize_last_touched_string, parse_last_touched,
            find_profile_folder, get_password};
use errors::{ThecaError, GenericError};
use crypt::{encrypt, decrypt, password_to_key};

pub use self::libc::{
    STDIN_FILENO,
    STDOUT_FILENO,
    STDERR_FILENO
};

#[macro_use] pub mod errors;
pub mod lineformat;
pub mod utils;
pub mod crypt;

#[derive(RustcDecodable, Show, Clone)]
pub struct Args {
    pub cmd_new_profile: bool,
    pub cmd_search: bool,
    pub cmd_add: bool,
    pub cmd_edit: bool,
    pub cmd_del: bool,
    pub cmd_clear: bool,
    pub cmd_info: bool,
    pub cmd_transfer: bool,
    pub arg_id: Vec<usize>,
    pub flag_v: bool,
    cmd__: bool,
    arg_name: String,
    arg_pattern: String,
    arg_title: String,
    flag_profile_folder: String,
    flag_p: String,
    flag_regex: bool,
    flag_body: bool,
    flag_reverse: bool,
    flag_encrypted: bool,
    flag_key: String,
    flag_c: bool,
    flag_l: usize,
    flag_datesort: bool,
    flag_started: bool,
    flag_urgent: bool,
    flag_none: bool,
    flag_b: String,
    flag_editor: bool,
    flag_h: bool,
    flag_y: bool,
    flag_append: String,
    flag_prepend: String
}

static NOSTATUS: &'static str = "";
static STARTED: &'static str = "Started";
static URGENT: &'static str = "Urgent";

static DATEFMT: &'static str = "%F %T %z";
static DATEFMT_SHORT: &'static str = "%F %T";

#[derive(RustcDecodable, RustcEncodable, Clone)]
pub struct ThecaItem {
    id: usize,
    title: String,
    status: String,
    body: String,
    last_touched: String
}

impl ThecaItem {
    fn print(&self, line_format: &LineFormat, body_search: bool) -> Result<(), ThecaError> {
        let column_seperator: String = repeat(' ').take(line_format.colsep).collect();
        print!("{}", format_field(&self.id.to_string(), line_format.id_width, false));
        print!("{}", column_seperator);
        let mut title_str = self.title.to_string();
        if !self.body.is_empty() && !body_search {
            title_str = "(+) ".to_string()+&*title_str;
        }
        print!("{}", format_field(&title_str, line_format.title_width, true));
        print!("{}", column_seperator);
        if line_format.status_width != 0 {
            print!("{}", format_field(&self.status, line_format.status_width, false));
            print!("{}", column_seperator);
        }
        print!("{}", format_field(&try!(localize_last_touched_string(&*self.last_touched)), line_format.touched_width, false));
        print!("\n");
        if body_search {
            for l in self.body.lines() {
                println!("\t{}", l);
            }
        }
        Ok(())
    }
}

#[derive(RustcDecodable, RustcEncodable)]
pub struct ThecaProfile {
    encrypted: bool,
    pub notes: Vec<ThecaItem>
}

impl ThecaProfile {
    pub fn new(args: &Args) -> Result<(ThecaProfile, u64), ThecaError> {
        if args.cmd_new_profile {
            let profile_path = try!(find_profile_folder(args));
            // if the folder doesn't exist, make it yo!
            if !profile_path.exists() {
                if !args.flag_y {
                    println!(
                        "{} doesn't exist, would you like to create it?",
                        profile_path.display()
                    );
                    if !try!(get_yn_input()) {specific_fail!("ok bye ♥".to_string());}
                }
                try!(mkdir(&profile_path, USER_RWX));
            }
            Ok((ThecaProfile {
                encrypted: args.flag_encrypted,
                notes: vec![]
            }, 0u64))
        } else {
            // set profile folder
            let mut profile_path = try!(find_profile_folder(args));

            // set profile name
            profile_path.push(args.flag_p.to_string() + ".json");
            
            // attempt to read profile
            match profile_path.is_file() {
                false => {
                    if profile_path.exists() {
                        specific_fail!(format!(
                            "{} is not a file.",
                            profile_path.display()
                        ));
                    } else {
                        specific_fail!(format!(
                            "{} does not exist.",
                            profile_path.display()
                        ));
                    }
                }
                true => {
                    let mut file = try!(File::open_mode(&profile_path, Open, Read));
                    let contents_buf = try!(file.read_to_end());
                    // decrypt the file if flag_encrypted
                    let contents = match args.flag_encrypted {
                        false => try!(String::from_utf8(contents_buf)),
                        true => {
                            let (key, iv) = password_to_key(&*args.flag_key);
                                try!(String::from_utf8(try!(decrypt(
                                    &*contents_buf,
                                    &*key,
                                    &*iv
                                ))))
                        }
                    };
                    let decoded: ThecaProfile = match json::decode(&*contents) {
                        Ok(s) => s,
                        Err(_) => specific_fail!(format!(
                            "Invalid JSON in {}",
                            profile_path.display()
                        ))
                    };
                    Ok((decoded, try!(profile_path.stat()).modified))
                }
            }
        }
    }

    pub fn clear(&mut self, args: &Args) -> Result<(), ThecaError> {
        println!("are you sure you want to delete all the notes in this profile?");
        if !args.flag_y && !try!(get_yn_input()) {specific_fail!("ok bye ♥".to_string());}
        self.notes.truncate(0);
        Ok(())
    }

    pub fn save_to_file(&mut self, args: &Args, fingerprint: &u64) -> Result<(), ThecaError> {
        // set profile folder
        let mut profile_path = try!(find_profile_folder(args));

        // set file name
        match args.cmd_new_profile {
            true => profile_path.push(args.arg_name.to_string() + ".json"),
            false => profile_path.push(args.flag_p.to_string() + ".json")
        }

        println!("{}", fingerprint);
        if fingerprint > &0u64 {
            let new_fingerprint = try!(profile_path.stat()).modified;
            if &new_fingerprint != fingerprint {
                println!("changes have been made to the profile '{}' on disk since it was loaded, would you like to attempt to merge them?", args.flag_p);
                if !args.flag_y && !try!(get_yn_input()) {specific_fail!("ok bye ♥".to_string());}
                // FIXME
            }
        }

        // open file
        let mut file = try!(File::open_mode(&profile_path, Truncate, Write));

        // encode to buffer
        let mut buffer: Vec<u8> = Vec::new();
        {
            let mut encoder = json::PrettyEncoder::new(&mut buffer);
            try!(self.encode(&mut encoder));
        }

        // encrypt json if its an encrypted profile
        if self.encrypted {
            let (key, iv) = password_to_key(&*args.flag_key);
            buffer = try!(encrypt(
                &*buffer,
                &*key,
                &*iv
            ));
        }

        // write buffer to file
        try!(file.write(&*buffer));

        Ok(())
    }

    pub fn import_note(&mut self, note: ThecaItem) -> Result<(), ThecaError> {
        let new_id = match self.notes.last() {
            Some(n) => n.id,
            None => 0
        };
        self.notes.push(ThecaItem {
            id: new_id + 1,
            title: note.title.clone(),
            status: note.status.clone(),
            body: note.body.clone(),
            last_touched: try!(strftime(DATEFMT, &now()))
        });
        Ok(())
    }

    pub fn transfer_note(&mut self, args: &Args) -> Result<(), ThecaError> {
        if args.flag_p == args.arg_name {
            specific_fail!(format!(
                "cannot transfer a note from a profile to itself ({} -> {})",
                args.flag_p,
                args.arg_name
            ));
        }

        let mut trans_args = args.clone();
        trans_args.flag_p = args.arg_name.clone();
        let (mut trans_profile, trans_fingerprint) = try!(ThecaProfile::new(&trans_args));

        match self.notes.iter().find(|n| n.id == args.arg_id[0])
                        .map(|n| trans_profile.import_note(n.clone())).is_some() {
            true =>  {
                match self.notes.iter().position(|n| n.id == args.arg_id[0])
                                   .map(|e| self.notes.remove(e)).is_some() {
                    true => try!(trans_profile.save_to_file(&trans_args, &trans_fingerprint)),
                    false => specific_fail!(format!(
                        "couldn't remove note {} in {}, aborting nothing will be saved",
                        args.arg_id[0],
                        args.flag_p
                    ))
                };
            },
            false => specific_fail!(format!(
                "could not transfer note {} from {} -> {}",
                args.arg_id[0],
                args.flag_p,
                args.arg_name
            ))
        };
        Ok(())
    }

    pub fn add_item(&mut self, args: &Args) -> Result<(), ThecaError> {
        let title = args.arg_title.replace("\n", "").to_string();
        let status = if args.flag_started {
            STARTED.to_string()
        } else if args.flag_urgent {
            URGENT.to_string()
        } else {
            NOSTATUS.to_string()
        };
        let body = if !args.flag_b.is_empty() {
            args.flag_b.to_string()
        } else if args.flag_editor {
            try!(drop_to_editor(&"".to_string()))
        } else if args.cmd__ {
            try!(stdin().lock().read_to_string())
        } else {
            "".to_string()
        };
        let new_id = match self.notes.last() {
            Some(n) => n.id,
            None => 0
        };
        self.notes.push(ThecaItem {
            id: new_id + 1,
            title: title,
            status: status,
            body: body,
            last_touched: try!(strftime(DATEFMT, &now()))
        });
        println!("note added");
        Ok(())
    }

    pub fn delete_item(&mut self, id: &usize) {
        let remove = self.notes.iter()
            .position(|n| n.id == *id)
            .map(|e| self.notes.remove(e))
            .is_some();
        match remove {
            true => {
                println!("note {} removed", id);
            }
            false => {
                println!("note {} doesn't exist", id);
            }
        }
    }

    pub fn edit_item(&mut self, args: &Args) -> Result<(), ThecaError> {
        let id = args.arg_id[0];
        let item_pos: usize = match self.notes.iter()
                                              .position(|n| n.id == id) {
                Some(i) => i,
                None => specific_fail!(format!("note {} doesn't exist", id))
            };
        if !args.arg_title.is_empty() {
            // change title
            self.notes[item_pos].title = args.arg_title.replace("\n", "").to_string();
        } else if !args.flag_prepend.is_empty() || !args.flag_append.is_empty() {
            self.notes[item_pos].title = format!(
                "{}{}{}",
                args.flag_prepend,
                self.notes[item_pos].title,
                args.flag_append
            );
        }
        if args.flag_started || args.flag_urgent || args.flag_none {
            // change status
            if args.flag_started {
                self.notes[item_pos].status = STARTED.to_string();
            } else if args.flag_urgent {
                self.notes[item_pos].status = URGENT.to_string();
            } else if args.flag_none {
                self.notes[item_pos].status = NOSTATUS.to_string();
            }
        }
        if !args.flag_b.is_empty() || args.flag_editor || args.cmd__ {
            // change body
            if !args.flag_b.is_empty() {
                self.notes[item_pos].body = args.flag_b.to_string();
            } else if args.flag_editor {
                if args.flag_encrypted && !args.flag_y {
                    // leak to disk warning
                    println!(
                        "{}\n{}\n{}\n{}\n{}",
                        "*******************************************************************",
                        "* Warning: this will write the decrypted note to disk temporarily *",
                        "* for editing, it will be deleted when you are done, but this     *",
                        "* increases the chance that it may be recovered at a later date.  *",
                        "*******************************************************************");
                    println!("Do you want to continue?");
                    if !try!(get_yn_input()) {specific_fail!("ok, bye".to_string());}
                }
                let new_body = try!(drop_to_editor(&self.notes[item_pos].body));
                if self.notes[item_pos].body != new_body {
                    self.notes[item_pos].body = new_body;
                }
            } else if args.cmd__ {
                try!(stdin().lock().read_to_string());
            }
        }
        // update last_touched
        self.notes[item_pos].last_touched = try!(strftime(DATEFMT, &now()));
        println!("edited");
        Ok(())
    }

    pub fn stats(&mut self, args: &Args) -> Result<(), ThecaError> {
        let no_s = self.notes.iter().filter(|n| n.status == "").count();
        let started_s = self.notes.iter().filter(|n| n.status == "Started").count();
        let urgent_s = self.notes.iter().filter(|n| n.status == "Urgent").count();
        let tty = istty(STDOUT_FILENO);
        let min = match self.notes.iter().min_by(|n| match parse_last_touched(&*n.last_touched) {
            Ok(o) => o,
            Err(_) => now() // FIXME
        }) {
            Some(n) => try!(localize_last_touched_string(&*n.last_touched)),
            None => specific_fail!("last_touched is not properly formated".to_string())
        };
        let max = match self.notes.iter().max_by(|n| match parse_last_touched(&*n.last_touched) {
            Ok(o) => o,
            Err(_) => now() // FIXME
        }) {
            Some(n) => try!(localize_last_touched_string(&*n.last_touched)),
            None => specific_fail!("last_touched is not properly formated".to_string())
        };
        try!(pretty_line("name: ", &format!("{}\n", args.flag_p), tty));
        try!(pretty_line("encrypted: ", &format!("{}\n", self.encrypted), tty));
        try!(pretty_line("notes: ", &format!("{}\n", self.notes.len()), tty));
        try!(pretty_line("statuses: ", &format!(
            "none: {}, started: {}, urgent: {}\n",
            no_s,
            started_s,
            urgent_s
        ), tty));
        try!(pretty_line("note ages: ", &format!("oldest: {}, newest: {}\n", min, max), tty));
        Ok(())
    }

    pub fn view_item(&mut self, args: &Args) -> Result<(), ThecaError> {
        let id = args.arg_id[0];
        let note_pos = match self.notes.iter().position(|n| n.id == id) {
            Some(i) => i,
            None => specific_fail!(format!("note {} doesn't exist", id))
        };
        let tty = istty(STDOUT_FILENO);

        match args.flag_c {
            true => {
                try!(pretty_line("id: ", &format!(
                    "{}\n",
                    self.notes[note_pos].id),
                    tty
                ));
                try!(pretty_line("title: ", &format!(
                    "{}\n",
                    self.notes[note_pos].title),
                    tty
                ));
                if !self.notes[note_pos].status.is_empty() {
                    try!(pretty_line("status: ", &format!(
                        "{}\n",
                        self.notes[note_pos].status),
                        tty
                    ));
                }
                try!(pretty_line(
                    "last touched: ",
                    &format!("{}\n", try!(localize_last_touched_string(&*self.notes[note_pos].last_touched))),
                    tty
                ));
            },
            false => {
                try!(pretty_line("id\n--\n", &format!(
                    "{}\n\n",
                    self.notes[note_pos].id),
                    tty
                ));
                try!(pretty_line("title\n-----\n", &format!(
                    "{}\n\n",
                    self.notes[note_pos].title),
                    tty
                ));
                if !self.notes[note_pos].status.is_empty() {
                    try!(pretty_line(
                        "status\n------\n",
                        &format!("{}\n\n", self.notes[note_pos].status),
                        tty
                    ));
                }
                try!(pretty_line(
                    "last touched\n------------\n",
                    &format!("{}\n\n", try!(localize_last_touched_string(&*self.notes[note_pos].last_touched))),
                    tty
                ));
            }
        };

        // body
        if !self.notes[note_pos].body.is_empty() {
            match args.flag_c {
                true => {
                    try!(pretty_line("body: ", &format!(
                        "{}\n",
                        self.notes[note_pos].body),
                        tty
                    ));
                },
                false => {
                    try!(pretty_line("body\n----\n", &format!(
                        "{}\n\n",
                        self.notes[note_pos].body),
                        tty
                    ));
                }
            };
        }
        Ok(())
    }

    pub fn list_items(&mut self, args: &Args) -> Result<(), ThecaError> {
        if self.notes.len() > 0 {
            try!(sorted_print(&mut self.notes.clone(), args));
        } else {
            println!("this profile is empty");
        }
        Ok(())
    }

    pub fn search_items(&mut self, args: &Args) -> Result<(), ThecaError> {
        let pattern = &*args.arg_pattern;
        let notes: Vec<ThecaItem> = match args.flag_regex {
            true => {
                let re = match Regex::new(pattern) {
                    Ok(r) => r,
                    Err(e) => specific_fail!(format!("regex error: {}.", e.msg))
                };
                self.notes.iter().filter(|n| match args.flag_body {
                    true => re.is_match(&*n.body),
                    false => re.is_match(&*n.title)
                }).map(|n| n.clone()).collect()
            },
            false => {
                self.notes.iter().filter(|n| match args.flag_body {
                    true => n.body.contains(pattern),
                    false => n.title.contains(pattern)
                }).map(|n| n.clone()).collect()
            }
        };
        if notes.len() > 0 {
            try!(sorted_print(&mut notes.clone(), args));
        } else {
            println!("nothing found");
        }
        Ok(())
    }
}

pub fn setup_args(args: &mut Args) -> Result<(), ThecaError> {
    match getenv("THECA_DEFAULT_PROFILE") {
        Some(val) => {
            if args.flag_p.is_empty() {
                args.flag_p = val;
            }
        },
        None => ()
    };

    match getenv("THECA_PROFILE_FOLDER") {
        Some(val) => {
            if args.flag_profile_folder.is_empty() {
                args.flag_profile_folder = val;
            }
        },
        None => ()
    };

    // if key is provided but --encrypted not set, it prob should be
    if !args.flag_key.is_empty() && !args.flag_encrypted {
        args.flag_encrypted = true;
    }

    // if profile is encrypted try to set the key
    if args.flag_encrypted && args.flag_key.is_empty() && !args.flag_y {
        args.flag_key = try!(get_password());
    }

    if args.flag_p.is_empty() {
        args.flag_p = "default".to_string();
    }

    Ok(())
}