#![feature(try_from, const_fn, duration_as_u128, nll)]
#![allow(unused_variables, unused_imports, unused_mut)]

use std::iter::Iterator;
use std::fs::{ File, OpenOptions };
use std::io::{ Bytes, Read, Seek, SeekFrom };


// https://en.wikipedia.org/wiki/Netpbm_format#File_format_description

pub const PBM_ASCII_MAGIC_NUMBER: [u8; 2]  = [80, 49]; // b"P1"
pub const PGM_ASCII_MAGIC_NUMBER: [u8; 2]  = [80, 50]; // b"P2"
pub const PPM_ASCII_MAGIC_NUMBER: [u8; 2]  = [80, 51]; // b"P3"

pub const PBM_BINARY_MAGIC_NUMBER: [u8; 2] = [80, 52]; // b"P4"
pub const PGM_BINARY_MAGIC_NUMBER: [u8; 2] = [80, 53]; // b"P5"
pub const PPM_BINARY_MAGIC_NUMBER: [u8; 2] = [80, 54]; // b"P6"

pub const PAM_BINARY_MAGIC_NUMBER: [u8; 2] = [80, 55]; // b"P7"

pub const LF: char = '\n';
pub const CR: char = '\r';


#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Lines<R: Read + Seek> {
    pub handle: R,
}

impl<RS: Read + Seek> Iterator for Lines<RS> {
    type Item = Vec<u8>;

    fn next(&mut self) -> Option<Self::Item> {
        let mut buffer: [u8; 1] = [0u8; 1];
        let mut line: Vec<u8> = Vec::new();

        loop {
            if let Ok(amt) = self.handle.read(&mut buffer) {
                if amt == 0 {
                    return if line.len() > 0 { Some(line) } else { None };
                }

                let byte = buffer[0];
                let c = byte as char;

                // https://en.wikipedia.org/wiki/Whitespace_character
                // https://doc.rust-lang.org/beta/reference/whitespace.html
                // https://internals.rust-lang.org/t/should-bufread-lines-et-al-recognize-more-than-just-lf/1735/11
                if c.is_whitespace() {
                    if c == LF {
                        // \n\r
                        if let Ok(amt) = self.handle.read(&mut buffer) {
                            if amt == 1 {
                                let byte = buffer[0];
                                let c = byte as char;
                                if c != CR {
                                    // back
                                    let pos = self.handle.seek(SeekFrom::Current(0)).unwrap();
                                    self.handle.seek(SeekFrom::Start(pos - 1)).unwrap();
                                }
                            }
                        }
                    } else if c == CR {
                        // \r\n
                        if let Ok(amt) = self.handle.read(&mut buffer) {
                            if amt == 1 {
                                let byte = buffer[0];
                                let c = byte as char;
                                if c != LF {
                                    // back
                                    let pos = self.handle.seek(SeekFrom::Current(0)).unwrap();
                                    self.handle.seek(SeekFrom::Start(pos - 1)).unwrap();
                                }
                            }
                        }
                    }

                    return if line.len() > 0 { Some(line) } else { None };
                } else {
                    line.push(byte);
                }

            } else {
                return if line.len() > 0 { Some(line) } else { None };
            }
        }
    }
}



fn main() {
    let filepath = "output.pam";
    let mut file = File::open(filepath).unwrap();
    let mut reader = Lines { handle: file };
    
    let mut idx = 0u8;
    for line in reader {
        println!("Line-{}: {:?}", idx, vec![0u8; 1]);
        idx += 1;
    }
}

