use crate::block::Block;
use crate::transaction::{CellOutput, OutPoint, Transaction};
use crate::Capacity;
use fnv::FnvHashMap;
use numext_fixed_hash::H256;

#[derive(Clone, Eq, PartialEq, Debug)]
pub struct CellMeta {
    pub cell_output: CellOutput,
    pub block_number: Option<u64>,
    pub cellbase: bool,
}

impl CellMeta {
    pub fn is_cellbase(&self) -> bool {
        self.cellbase
    }

    pub fn capacity(&self) -> Capacity {
        self.cell_output.capacity
    }
}

#[derive(Clone, PartialEq, Debug)]
pub enum CellStatus {
    /// Cell exists and has not been spent.
    Live(CellMeta),
    /// Cell exists and has been spent.
    Dead,
    /// Cell does not exist.
    Unknown,
}

impl CellStatus {
    pub fn live_output(
        cell_output: CellOutput,
        block_number: Option<u64>,
        cellbase: bool,
    ) -> CellStatus {
        CellStatus::Live(CellMeta {
            cell_output,
            block_number,
            cellbase,
        })
    }

    pub fn is_live(&self) -> bool {
        match *self {
            CellStatus::Live(_) => true,
            _ => false,
        }
    }

    pub fn is_dead(&self) -> bool {
        self == &CellStatus::Dead
    }

    pub fn is_unknown(&self) -> bool {
        self == &CellStatus::Unknown
    }

    pub fn get_live_output(&self) -> Option<&CellOutput> {
        match self {
            CellStatus::Live(cell_meta) => Some(&cell_meta.cell_output),
            _ => None,
        }
    }
}

/// Transaction with resolved input cells.
#[derive(Debug)]
pub struct ResolvedTransaction {
    pub transaction: Transaction,
    pub dep_cells: Vec<CellStatus>,
    pub input_cells: Vec<CellStatus>,
}

pub trait CellProvider {
    fn cell(&self, out_point: &OutPoint) -> CellStatus;

    fn get_cell_status(&self, out_point: &OutPoint) -> CellStatus {
        self.cell(out_point)
    }

    fn resolve_transaction(&self, transaction: &Transaction) -> ResolvedTransaction {
        // setup empty input cells for cellbase
        let input_cells = if transaction.is_cellbase() {
            Vec::new()
        } else {
            transaction
                .input_pts()
                .iter()
                .map(|input| self.get_cell_status(input))
                .collect()
        };

        let dep_cells = transaction
            .dep_pts()
            .iter()
            .map(|dep| self.get_cell_status(dep))
            .collect();

        ResolvedTransaction {
            transaction: transaction.clone(),
            input_cells,
            dep_cells,
        }
    }
}

pub struct OverlayCellProvider<'a> {
    overlay: &'a CellProvider,
    cell_provider: &'a CellProvider,
}

impl<'a> OverlayCellProvider<'a> {
    pub fn new(overlay: &'a CellProvider, cell_provider: &'a CellProvider) -> Self {
        Self {
            overlay,
            cell_provider,
        }
    }
}

impl<'a> CellProvider for OverlayCellProvider<'a> {
    fn cell(&self, out_point: &OutPoint) -> CellStatus {
        match self.overlay.get_cell_status(out_point) {
            CellStatus::Live(co) => CellStatus::Live(co),
            CellStatus::Dead => CellStatus::Dead,
            CellStatus::Unknown => self.cell_provider.get_cell_status(out_point),
        }
    }
}

pub struct BlockCellProvider<'a> {
    output_indices: FnvHashMap<H256, usize>,
    duplicate_inputs_counter: FnvHashMap<&'a OutPoint, usize>,
    block: &'a Block,
}

impl<'a> BlockCellProvider<'a> {
    pub fn new(block: &'a Block) -> Self {
        let mut duplicate_inputs_counter = FnvHashMap::default();

        let output_indices = block
            .transactions()
            .iter()
            .enumerate()
            .map(|(idx, tx)| {
                tx.inputs().iter().for_each(|input| {
                    duplicate_inputs_counter
                        .entry(&input.previous_output)
                        .and_modify(|c| *c += 1)
                        .or_default();
                });
                (tx.hash(), idx)
            })
            .collect();

        Self {
            output_indices,
            duplicate_inputs_counter,
            block,
        }
    }
}

impl<'a> CellProvider for BlockCellProvider<'a> {
    fn cell(&self, out_point: &OutPoint) -> CellStatus {
        if let Some(true) = self
            .duplicate_inputs_counter
            .get(out_point)
            .map(|counter| *counter > 0)
        {
            CellStatus::Dead
        } else if let Some(i) = self.output_indices.get(&out_point.tx_hash) {
            match self.block.transactions()[*i]
                .outputs()
                .get(out_point.index as usize)
            {
                Some(x) => {
                    CellStatus::live_output(x.clone(), Some(self.block.header().number()), *i == 0)
                }
                None => CellStatus::Unknown,
            }
        } else {
            CellStatus::Unknown
        }
    }
}

pub struct TransactionCellProvider<'a> {
    duplicate_inputs_counter: FnvHashMap<&'a OutPoint, usize>,
}

impl<'a> TransactionCellProvider<'a> {
    pub fn new(transaction: &'a Transaction) -> Self {
        let mut duplicate_inputs_counter = FnvHashMap::default();

        transaction.inputs().iter().for_each(|input| {
            duplicate_inputs_counter
                .entry(&input.previous_output)
                .and_modify(|c| *c += 1)
                .or_default();
        });

        Self {
            duplicate_inputs_counter,
        }
    }
}

impl<'a> CellProvider for TransactionCellProvider<'a> {
    fn cell(&self, out_point: &OutPoint) -> CellStatus {
        if let Some(true) = self
            .duplicate_inputs_counter
            .get(out_point)
            .map(|counter| *counter > 0)
        {
            CellStatus::Dead
        } else {
            CellStatus::Unknown
        }
    }
}

impl ResolvedTransaction {
    pub fn is_cellbase(&self) -> bool {
        self.input_cells.is_empty()
    }

    pub fn fee(&self) -> ::occupied_capacity::Result<Capacity> {
        self.inputs_capacity().and_then(|x| {
            self.transaction.outputs_capacity().and_then(|y| {
                if x > y {
                    x.safe_sub(y)
                } else {
                    Ok(Capacity::zero())
                }
            })
        })
    }

    pub fn inputs_capacity(&self) -> ::occupied_capacity::Result<Capacity> {
        self.input_cells
            .iter()
            .filter_map(|cell_status| {
                if let CellStatus::Live(cell_meta) = cell_status {
                    Some(cell_meta.capacity())
                } else {
                    None
                }
            })
            .try_fold(Capacity::zero(), Capacity::safe_add)
    }
}

#[cfg(test)]
mod tests {
    use super::super::script::Script;
    use super::*;
    use crate::{capacity_bytes, Capacity};
    use numext_fixed_hash::H256;
    use std::collections::HashMap;

    struct CellMemoryDb {
        cells: HashMap<OutPoint, Option<CellMeta>>,
    }
    impl CellProvider for CellMemoryDb {
        fn cell(&self, o: &OutPoint) -> CellStatus {
            match self.cells.get(o) {
                Some(&Some(ref cell_meta)) => CellStatus::Live(cell_meta.clone()),
                Some(&None) => CellStatus::Dead,
                None => CellStatus::Unknown,
            }
        }
    }

    #[test]
    fn cell_provider_trait_works() {
        let mut db = CellMemoryDb {
            cells: HashMap::new(),
        };

        let p1 = OutPoint {
            tx_hash: H256::zero(),
            index: 1,
        };
        let p2 = OutPoint {
            tx_hash: H256::zero(),
            index: 2,
        };
        let p3 = OutPoint {
            tx_hash: H256::zero(),
            index: 3,
        };
        let o = CellMeta {
            block_number: Some(1),
            cell_output: CellOutput {
                capacity: capacity_bytes!(2),
                data: vec![],
                lock: Script::default(),
                type_: None,
            },
            cellbase: false,
        };

        db.cells.insert(p1.clone(), Some(o.clone()));
        db.cells.insert(p2.clone(), None);

        assert_eq!(CellStatus::Live(o), db.get_cell_status(&p1));
        assert_eq!(CellStatus::Dead, db.get_cell_status(&p2));
        assert_eq!(CellStatus::Unknown, db.get_cell_status(&p3));
    }
}
