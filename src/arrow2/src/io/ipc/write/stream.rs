//! Arrow IPC File and Stream Writers
//!
//! The `FileWriter` and `StreamWriter` have similar interfaces,
//! however the `FileWriter` expects a reader that supports `Seek`ing

use std::io::Write;

use super::super::IpcField;
use super::common::{encode_chunk, DictionaryTracker, EncodedData, WriteOptions};
use super::common_sync::{write_continuation, write_message};
use super::{default_ipc_fields, schema_to_bytes};

use crate::array::Array;
use crate::chunk::Chunk;
use crate::datatypes::*;
use crate::error::{Error, Result};

/// Arrow stream writer
///
/// The data written by this writer must be read in order. To signal that no more
/// data is arriving through the stream call [`self.finish()`](StreamWriter::finish);
///
/// For a usage walkthrough consult [this example](https://github.com/jorgecarleitao/arrow2/tree/main/examples/ipc_pyarrow).
pub struct StreamWriter<W: Write> {
    /// The object to write to
    writer: W,
    /// IPC write options
    write_options: WriteOptions,
    /// Whether the stream has been finished
    finished: bool,
    /// Keeps track of dictionaries that have been written
    dictionary_tracker: DictionaryTracker,

    ipc_fields: Option<Vec<IpcField>>,

    bytes_written: usize,
}

impl<W: Write> StreamWriter<W> {
    /// Creates a new [`StreamWriter`]
    pub fn new(writer: W, write_options: WriteOptions) -> Self {
        Self {
            writer,
            write_options,
            finished: false,
            dictionary_tracker: DictionaryTracker {
                dictionaries: Default::default(),
                cannot_replace: false,
            },
            ipc_fields: None,
            bytes_written: 0,
        }
    }

    /// Starts the stream by writing a Schema message to it.
    /// Use `ipc_fields` to declare dictionary ids in the schema, for dictionary-reuse
    pub fn start(&mut self, schema: &Schema, ipc_fields: Option<Vec<IpcField>>) -> Result<()> {
        self.ipc_fields = Some(if let Some(ipc_fields) = ipc_fields {
            ipc_fields
        } else {
            default_ipc_fields(&schema.fields)
        });

        let encoded_message = EncodedData {
            ipc_message: schema_to_bytes(schema, self.ipc_fields.as_ref().unwrap()),
            arrow_data: vec![],
        };
        let (metadata_len, data_len) = write_message(&mut self.writer, &encoded_message)?;
        self.bytes_written += metadata_len + data_len;
        Ok(())
    }

    /// Writes [`Chunk`] to the stream
    pub fn write(
        &mut self,
        columns: &Chunk<Box<dyn Array>>,
        ipc_fields: Option<&[IpcField]>,
    ) -> Result<()> {
        if self.finished {
            return Err(Error::Io(std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                "Cannot write to a finished stream".to_string(),
            )));
        }

        // we can't make it a closure because it borrows (and it can't borrow mut and non-mut below)
        #[allow(clippy::or_fun_call)]
        let fields = ipc_fields.unwrap_or(self.ipc_fields.as_ref().unwrap());

        let (encoded_dictionaries, encoded_message) = encode_chunk(
            columns,
            fields,
            &mut self.dictionary_tracker,
            &self.write_options,
        )?;

        for encoded_dictionary in encoded_dictionaries {
            let (metadata_len, data_len) = write_message(&mut self.writer, &encoded_dictionary)?;
            self.bytes_written += metadata_len + data_len;
        }

        let (metadata_len, data_len) = write_message(&mut self.writer, &encoded_message)?;
        self.bytes_written += metadata_len + data_len;
        Ok(())
    }

    pub fn bytes_written(&self) -> usize {
        self.bytes_written
    }

    /// Write continuation bytes, and mark the stream as done
    pub fn finish(&mut self) -> Result<()> {
        write_continuation(&mut self.writer, 0)?;

        self.finished = true;

        Ok(())
    }

    /// Consumes itself, returning the inner writer.
    pub fn into_inner(self) -> W {
        self.writer
    }
}
